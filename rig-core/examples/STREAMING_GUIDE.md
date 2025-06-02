# Streaming Support in Rig Pipelines - Complete Implementation Guide

## Table of Contents
- [Overview](#overview)
- [Architecture](#architecture)
- [Core Components](#core-components)
- [Usage Patterns](#usage-patterns)
- [Implementation Details](#implementation-details)
- [Rust Concepts Explained](#rust-concepts-explained)
- [Integration Examples](#integration-examples)
- [Best Practices](#best-practices)

## Overview

This guide explains the streaming support implementation in rig-core pipelines. The implementation allows you to:

1. **Get real-time streaming chunks** from LLM responses via channels (like `tokio::sync::mpsc`)
2. **Still receive the final aggregated response** through the regular `Op` interface
3. **Seamlessly integrate** with existing pipeline patterns
4. **Use any sender mechanism** through a generic `StreamingSender` trait

### Key Benefits
- ✅ Real-time streaming without changing existing pipeline architecture
- ✅ Generic sender interface (works with tokio channels, custom senders, etc.)
- ✅ Maintains compatibility with existing `Op` trait
- ✅ Clean separation of streaming and final response handling

## Architecture

The streaming implementation uses a **"dual output" approach**:

```text
Input String → StreamingPromptWithSender → 
                     ↓                ↓
              Streaming Chunks    Final String
              (via Sender)        (via Op return)
```

### Core Design Principles

1. **Non-invasive**: Doesn't change existing `Op` trait or pipeline behavior
2. **Generic**: Works with any sender that implements `StreamingSender`
3. **Composable**: Integrates seamlessly with existing pipeline combinators
4. **Type-safe**: Full Rust type safety with appropriate trait bounds

## Core Components

### 1. StreamingSender Trait

```rust
pub trait StreamingSender: Send + Sync {
    /// Send a chunk of text
    fn send_chunk(&self, chunk: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// Signal that streaming is complete (optional)
    fn close(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}
```

**Purpose**: Abstraction over different sending mechanisms
**Key Features**:
- Generic over any sender type
- Error handling for send failures
- Optional close notification

### 2. StreamingPromptWithSender Struct

```rust
pub struct StreamingPromptWithSender<P, In, S> {
    prompt: P,    // The agent/model to prompt
    sender: S,    // The streaming sender
    _in: std::marker::PhantomData<In>,
}
```

**Purpose**: Main operation that handles both streaming and final response
**Type Parameters**:
- `P`: The promptable agent/model
- `In`: Input type (must convert to String)
- `S`: The sender type (must implement StreamingSender)

### 3. Op Implementation

```rust
impl<M, In, S> Op for StreamingPromptWithSender<Agent<M>, In, S>
where
    M: StreamingCompletionModel + Send + Sync,
    M::StreamingResponse: Clone + Unpin + Send + Sync,
    In: Into<String> + Send + Sync,
    S: StreamingSender,
{
    type Input = In;
    type Output = Result<String, CompletionError>;

    async fn call(&self, input: Self::Input) -> Self::Output {
        // Convert input to message
        let message = Message::user(input.into());
        
        // Start streaming
        let mut stream = self.prompt.stream_prompt(message).await?;
        
        let mut final_text = String::new();
        
        // Process each chunk
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(AssistantContent::Text(text)) => {
                    // Send chunk via sender (ignore errors)
                    let _ = self.sender.send_chunk(text.text.clone());
                    // Accumulate for final response
                    final_text.push_str(&text.text);
                }
                Ok(AssistantContent::ToolCall(_)) => continue,
                Err(e) => return Err(e),
            }
        }
        
        // Close streaming
        let _ = self.sender.close();
        
        Ok(final_text)
    }
}
```

**Key Behaviors**:
- Streams each chunk via the sender
- Accumulates all chunks into final response
- Handles errors gracefully
- Ignores sender errors (non-blocking)

## Usage Patterns

### Pattern 1: Traditional Pipeline → Streaming Pipeline

**Before (Traditional)**:
```rust
let pipeline = pipeline::new()
    .map(|input: String| format!("Tell me about: {}", input))
    .prompt(agent);

let result = pipeline.call("Rust programming").await?;
```

**After (Streaming)**:
```rust
let (tx, mut rx) = mpsc::unbounded_channel();

let pipeline = pipeline::new()
    .map(|input: String| format!("Tell me about: {}", input))
    .chain(streaming_prompt_with_sender(agent, tx));

// Handle streaming chunks
tokio::spawn(async move {
    while let Some(chunk) = rx.recv().await {
        print!("{}", chunk);
    }
});

let result = pipeline.call("Rust programming").await?;
```

### Pattern 2: External Processing with Custom Structs (Recommended)

**New Approach - Complete External Control**:
```rust
#[derive(Debug, Serialize, Deserialize)]
struct MyCustomData {
    timestamp: u64,
    content: String,
    chunk_id: usize,
    word_count: usize,
    session_id: String,
    // Your custom fields
}

let (tx, mut rx) = mpsc::unbounded_channel::<MyCustomData>();

let pipeline = pipeline::new()
    .map(|input: String| format!("Tell me about: {}", input))
    .chain(streaming_prompt(agent)); // Returns raw stream

// Handle your custom structured data
tokio::spawn(async move {
    while let Some(my_data) = rx.recv().await {
        // Your custom processing logic
        send_to_websocket(&my_data).await;
        save_to_database(&my_data).await;
    }
});

// Get the stream and process externally
let stream_response = pipeline.call("Rust programming").await?;
let mut stream = stream_response;

while let Some(chunk_result) = stream.next().await {
    match chunk_result {
        Ok(AssistantContent::Text(text)) => {
            // Create YOUR custom struct
            let my_data = MyCustomData {
                timestamp: now(),
                content: text.text,
                chunk_id: chunk_counter,
                word_count: text.text.split_whitespace().count(),
                session_id: session_id.clone(),
            };
            
            // Direct send of YOUR struct
            tx.send(my_data)?;
        }
        // Handle other content types...
    }
}
```

### Pattern 3: Inline Agent Building

```rust
let pipeline = pipeline::new()
    .map(|input: String| format!("Write a story about: {}", input))
    .chain(streaming_prompt_with_sender(
        client.agent(GEMINI_1_5_FLASH)
            .preamble("You are a creative writer")
            .build(),
        tx
    ));
```

### Pattern 4: Custom Sender Implementation

```rust
struct LoggingSender {
    file: Arc<Mutex<File>>,
}

impl StreamingSender for LoggingSender {
    fn send_chunk(&self, chunk: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut file = self.file.lock().unwrap();
        writeln!(file, "{}", chunk)?;
        Ok(())
    }
}
```

## Implementation Details

### Why Agent-Specific Implementation?

The `Op` implementation is currently specific to `Agent<M>` types because:

1. **StreamingPrompt Trait**: Only `Agent` implements the `StreamingPrompt` trait
2. **Type Safety**: Ensures we only accept streaming-capable models
3. **Future Extensibility**: Can add implementations for other streaming types

### Trait Bounds Explained

```rust
M: StreamingCompletionModel + Send + Sync,
M::StreamingResponse: Clone + Unpin + Send + Sync,
```

- `StreamingCompletionModel`: Model can stream responses
- `Send + Sync`: Thread-safe for async operations
- `Clone + Unpin`: Required for stream processing
- `M::StreamingResponse`: Associated type for streaming responses

### Error Handling Strategy

```rust
// Send chunk via sender (ignore errors)
let _ = self.sender.send_chunk(text.text.clone());
```

**Why ignore sender errors?**
- Streaming is "best effort" - don't fail the entire operation if sender fails
- Final response is still returned even if streaming fails
- User can handle sender errors in their sender implementation

## Rust Concepts Explained

### 1. Trait Objects and Generics

```rust
pub trait StreamingSender: Send + Sync {
    fn send_chunk(&self, chunk: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
```

- `Send + Sync`: Trait bounds for thread safety
- `Box<dyn std::error::Error + Send + Sync>`: Trait object for any error type
- `dyn`: Dynamic dispatch for different error types

### 2. PhantomData

```rust
_in: std::marker::PhantomData<In>,
```

- Used when struct needs to be generic over a type it doesn't actually store
- Zero-cost abstraction - no runtime overhead
- Helps the type checker track generic parameters

### 3. Associated Types vs Generics

```rust
M: StreamingCompletionModel,
M::StreamingResponse: Clone + Unpin,
```

- `M::StreamingResponse`: Associated type from the trait
- More constrained than generics - each model has one response type
- Better type inference and cleaner APIs

### 4. Async Traits and Futures

```rust
async fn call(&self, input: Self::Input) -> Self::Output
```

- `async fn` automatically returns `impl Future`
- `Send` bound ensures future can be moved between threads
- Required for tokio's async runtime

## Integration Examples

### Example 1: Basic Tokio Channel

```rust
use rig::pipeline::{self, streaming_prompt_with_sender, Op};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = gemini::Client::from_env();
    let agent = client.agent(GEMINI_1_5_FLASH).build();
    
    let (tx, mut rx) = mpsc::unbounded_channel();
    
    let pipeline = pipeline::new()
        .map(|input: String| format!("Explain: {}", input))
        .chain(streaming_prompt_with_sender(agent, tx));
    
    // Spawn task to handle streaming
    tokio::spawn(async move {
        while let Some(chunk) = rx.recv().await {
            print!("{}", chunk);
            std::io::stdout().flush().unwrap();
        }
    });
    
    let final_response = pipeline.call("quantum computing".to_string()).await?;
    println!("\n\nFinal response length: {}", final_response.len());
    
    Ok(())
}
```

### Example 2: Multiple Receivers

```rust
use tokio::sync::broadcast;

let (tx, _rx) = broadcast::channel(100);

// Multiple subscribers
let rx1 = tx.subscribe();
let rx2 = tx.subscribe();

// Wrapper to make broadcast sender work with StreamingSender
struct BroadcastSender(broadcast::Sender<String>);

impl StreamingSender for BroadcastSender {
    fn send_chunk(&self, chunk: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.0.send(chunk)?;
        Ok(())
    }
}

let pipeline = pipeline::new()
    .map(|input: String| format!("Analyze: {}", input))
    .chain(streaming_prompt_with_sender(agent, BroadcastSender(tx)));
```

### Example 3: WebSocket Streaming

```rust
use tokio_tungstenite::WebSocketStream;

struct WebSocketSender {
    ws: Arc<Mutex<WebSocketStream<TcpStream>>>,
}

impl StreamingSender for WebSocketSender {
    fn send_chunk(&self, chunk: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let ws = self.ws.clone();
        // In real implementation, you'd spawn a task or use a channel
        // to avoid blocking the main stream processing
        Ok(())
    }
}
```

## Best Practices

### 1. Error Handling

```rust
// Good: Handle receiver errors gracefully
tokio::spawn(async move {
    while let Some(chunk) = rx.recv().await {
        if let Err(e) = process_chunk(chunk).await {
            eprintln!("Failed to process chunk: {}", e);
            // Continue processing other chunks
        }
    }
});
```

### 2. Backpressure Management

```rust
// Use bounded channels to prevent memory issues
let (tx, rx) = mpsc::channel(100); // Bounded channel
```

### 3. Graceful Shutdown

```rust
// Implement close() for cleanup
impl StreamingSender for MyCustomSender {
    fn close(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self.cleanup_resources();
        Ok(())
    }
}
```

### 4. Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    
    struct TestSender {
        chunks: Arc<Mutex<Vec<String>>>,
    }
    
    impl StreamingSender for TestSender {
        fn send_chunk(&self, chunk: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.chunks.lock().unwrap().push(chunk);
            Ok(())
        }
    }
    
    #[tokio::test]
    async fn test_streaming_prompt() {
        let sender = TestSender { chunks: Arc::new(Mutex::new(Vec::new())) };
        let chunks_ref = sender.chunks.clone();
        
        // ... test implementation
        
        let chunks = chunks_ref.lock().unwrap();
        assert!(!chunks.is_empty());
    }
}
```

### 5. Performance Considerations

- Use `unbounded_channel()` for high-throughput scenarios
- Consider using `broadcast` channels for multiple consumers
- Implement custom senders for specialized use cases (logging, metrics, etc.)
- Be mindful of sender failures - they won't stop the pipeline

## Comparison with Alternatives

### Approach 1: Streaming-Only Response
```rust
// Returns only streaming, no final aggregated response
fn stream_only() -> impl Stream<Item = String>
```
❌ **Drawback**: Loses compatibility with existing Op-based pipelines

### Approach 2: Callback-Based
```rust
// Uses callbacks instead of channels
fn with_callback(callback: impl Fn(String))
```
❌ **Drawback**: Less flexible, harder to integrate with async code

### Approach 3: Our Implementation
```rust
// Dual output: streaming + final response
fn streaming_prompt_with_sender(agent, sender) -> impl Op
```
✅ **Benefits**: 
- Maintains Op compatibility
- Flexible sender interface
- Real-time streaming
- Final aggregated response

## Conclusion

This streaming implementation provides a clean, efficient way to add real-time streaming to rig pipelines while maintaining full compatibility with existing patterns. The generic `StreamingSender` trait allows for flexible integration with different streaming mechanisms, from simple tokio channels to complex distributed systems.

The key insight is the "dual output" approach: stream chunks in real-time while still providing the final aggregated response through the standard Op interface. This gives you the best of both worlds without compromising on the existing architecture. 