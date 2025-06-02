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

1. **Get raw streaming responses** from LLM models via the `streaming_prompt()` function
2. **Process chunks externally** with your own custom logic and data structures
3. **Send structured data** using direct `tx.send(MyCustomStruct)` calls
4. **Seamlessly integrate** with existing pipeline patterns

### Key Benefits
- ✅ Real-time streaming with complete external control
- ✅ Direct `tx.send(MyCustomStruct)` usage - no abstractions
- ✅ Maintains compatibility with existing `Op` trait  
- ✅ Clean separation: rig handles streaming, you handle processing

## Architecture

The streaming implementation uses an **"external processing" approach**:

```text
Input String → streaming_prompt(agent) → Raw Stream
                                           ↓
                              Your Processing Logic
                                           ↓
                              tx.send(MyCustomStruct)
```

### Core Design Principles

1. **Non-invasive**: Doesn't change existing `Op` trait or pipeline behavior
2. **External control**: You handle all processing and sending logic outside of rig
3. **Composable**: Integrates seamlessly with existing pipeline combinators
4. **Type-safe**: Full Rust type safety with appropriate trait bounds

## Core Components

### 1. StreamingPrompt Struct

```rust
pub struct StreamingPrompt<P, In> {
    prompt: P,
    _in: std::marker::PhantomData<In>,
}
```

**Purpose**: Operation that returns raw streaming response for external processing
**Type Parameters**:
- `P`: The promptable agent/model
- `In`: Input type (must convert to String)

### 2. Op Implementation

```rust
impl<M, In> Op for StreamingPrompt<Agent<M>, In>
where
    M: StreamingCompletionModel + Send + Sync,
    M::StreamingResponse: Clone + Unpin + Send + Sync,
    In: Into<String> + Send + Sync,
{
    type Input = In;
    type Output = Result<StreamingCompletionResponse<M::StreamingResponse>, CompletionError>;

    async fn call(&self, input: Self::Input) -> Self::Output {
        let message = Message::user(input.into());
        self.prompt.stream_prompt(message).await
    }
}
```

**Key Behaviors**:
- Converts input to user message
- Calls the agent's streaming method
- Returns raw stream for external processing
- No internal processing or sending - that's your responsibility

## Usage Patterns

### Pattern 1: Traditional Pipeline → External Streaming

**Before (Traditional)**:
```rust
let pipeline = pipeline::new()
    .map(|input: String| format!("Tell me about: {}", input))
    .prompt(agent);

let result = pipeline.call("Rust programming").await?;
```

**After (External Streaming)**:
```rust
#[derive(Debug, Serialize, Deserialize)]
struct MyCustomData {
    timestamp: u64,
    content: String,
    chunk_id: usize,
    // Your custom fields...
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
                // ... your custom fields
            };
            
            // Direct send of YOUR struct
            tx.send(my_data)?;
        }
        // Handle other content types...
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

### External Processing Strategy

```rust
// You get the raw stream and process it yourself
let stream_response = pipeline.call(input).await?;
let mut stream = stream_response;

while let Some(chunk_result) = stream.next().await {
    match chunk_result {
        Ok(AssistantContent::Text(text)) => {
            // YOUR custom processing logic here
            let my_data = MyCustomStruct {
                content: text.text,
                // ... your fields
            };
            tx.send(my_data)?;
        }
    }
}
```

**Why external processing?**
- Complete control over data structure and processing logic
- No abstraction overhead - direct `tx.send(MyCustomStruct)`
- Flexibility to handle chunks however you want
- Clean separation of concerns between rig and your application

## Detailed Implementation Changes

### Changes to `agent_ops.rs` - For Novice Rust Developers

This section explains exactly what code was added to enable external streaming processing, with detailed explanations of Rust concepts.

#### 1. New StreamingPrompt Struct

**What was added:**
```rust
/// Streaming prompt operation that returns the stream directly for external processing
pub struct StreamingPrompt<P, In> {
    prompt: P,
    _in: std::marker::PhantomData<In>,
}
```

**Rust concepts explained:**

**Generic Types (`<P, In>`):**
- `P` represents the type of the prompt/agent (like `Agent<GeminiModel>`)
- `In` represents the input type (like `String` or `&str`)
- Generics let the same struct work with different types while maintaining type safety

**PhantomData:**
```rust
_in: std::marker::PhantomData<In>,
```
- **What it is**: A zero-sized type that "marks" the struct as using type `In`
- **Why needed**: The struct needs to be generic over `In` for type safety, but doesn't actually store an `In` value
- **Runtime cost**: Zero - it completely disappears when compiled
- **Alternative**: Without PhantomData, Rust would complain about "unused type parameter"

**Example analogy**: It's like putting a label on a box saying "this box is for apples" even though the box is currently empty.

#### 2. Constructor Implementation

**What was added:**
```rust
impl<P, In> StreamingPrompt<P, In> {
    pub fn new(prompt: P) -> Self {
        Self {
            prompt,
            _in: std::marker::PhantomData,
        }
    }
}
```

**Rust concepts explained:**

**impl blocks:**
- `impl<P, In>` means "for any types P and In"
- This provides methods for the `StreamingPrompt` struct
- `Self` is a shorthand for `StreamingPrompt<P, In>`

**Constructor pattern:**
- `new()` is a Rust convention for constructors (not a language keyword)
- Takes ownership of `prompt` and moves it into the struct
- `std::marker::PhantomData` creates the zero-sized marker

#### 3. The Core Op Implementation

**What was added:**
```rust
impl<M, In> Op for StreamingPrompt<Agent<M>, In>
where
    M: StreamingCompletionModel + Send + Sync,
    M::StreamingResponse: Clone + Unpin + Send + Sync,
    In: Into<String> + Send + Sync,
{
    type Input = In;
    type Output = Result<crate::streaming::StreamingCompletionResponse<M::StreamingResponse>, CompletionError>;

    async fn call(&self, input: Self::Input) -> Self::Output {
        let message = Message::user(input.into());
        self.prompt.stream_prompt(message).await
    }
}
```

**Breaking this down for novice developers:**

**Trait Implementation:**
```rust
impl<M, In> Op for StreamingPrompt<Agent<M>, In>
```
- This says: "StreamingPrompt containing an Agent implements the Op trait"
- `Op` is rig's core interface for pipeline operations
- Only works with `Agent<M>` because only agents can stream in rig

**Where Clause (Constraints):**
```rust
where
    M: StreamingCompletionModel + Send + Sync,
    M::StreamingResponse: Clone + Unpin + Send + Sync,
    In: Into<String> + Send + Sync,
```

**Each constraint explained:**

1. **`M: StreamingCompletionModel`**
   - The model type `M` must implement streaming
   - Ensures we can call `.stream_prompt()` on it
   - **Real example**: `GeminiModel` implements this, but a non-streaming model wouldn't

2. **`M: Send + Sync`**
   - `Send`: Type can be moved between threads
   - `Sync`: Type can be shared between threads safely
   - **Why needed**: Async operations might run on different threads

3. **`M::StreamingResponse: Clone + Unpin + Send + Sync`**
   - `M::StreamingResponse`: The response type from streaming (associated type)
   - `Clone`: Must be cloneable (for stream processing)
   - `Unpin`: Can be moved in memory (required for async streams)
   - `Send + Sync`: Thread safety for async operations

4. **`In: Into<String> + Send + Sync`**
   - `Into<String>`: Input can be converted to String
   - **Examples**: `String`, `&str`, `Cow<str>` all implement `Into<String>`
   - **Flexibility**: Users can pass different string types

**Associated Types:**
```rust
type Input = In;
type Output = Result<crate::streaming::StreamingCompletionResponse<M::StreamingResponse>, CompletionError>;
```

- **`Input = In`**: Simple - input type is whatever `In` is
- **`Output = Result<...>`**: Returns either a streaming response or an error
  - **Success**: `StreamingCompletionResponse` (implements `Stream`)
  - **Error**: `CompletionError` if streaming fails to start

**The Core Logic:**
```rust
async fn call(&self, input: Self::Input) -> Self::Output {
    let message = Message::user(input.into());
    self.prompt.stream_prompt(message).await
}
```

**Step-by-step execution:**

1. **`input: Self::Input`** - Receive input (String, &str, etc.)
2. **`input.into()`** - Convert to String using `Into` trait
3. **`Message::user(...)`** - Wrap string as a user message in rig's format
4. **`self.prompt.stream_prompt(message)`** - Call agent's streaming method
5. **`.await`** - Wait for async operation (streaming setup)
6. **Return** - Either streaming response or error

#### 4. Factory Function

**What was added:**
```rust
/// Create a new streaming prompt operation that returns the stream for external processing
pub fn streaming_prompt<P, Input>(
    model: P
) -> agent_ops::StreamingPrompt<P, Input>
where
    P: Send + Sync,
    Input: Send + Sync,
{
    agent_ops::streaming_prompt(model)
}
```

**Why a factory function:**
- **Convenience**: Easier to call than `StreamingPrompt::new()`
- **Type inference**: Rust can often infer the generic types
- **API consistency**: Matches other rig functions like `prompt()`, `map()`, etc.

### Changes to `mod.rs` - Module Integration

**What was added:**
```rust
/// Create a new streaming prompt operation that returns the stream for external processing
pub fn streaming_prompt<P, Input>(
    model: P
) -> agent_ops::StreamingPrompt<P, Input>
where
    P: Send + Sync,
    Input: Send + Sync,
{
    agent_ops::streaming_prompt(model)
}
```

**Why this is needed:**
- **Public API**: Makes the function available outside the module
- **Re-export**: Users can call `pipeline::streaming_prompt()` instead of `agent_ops::streaming_prompt()`
- **API consistency**: Follows same pattern as other pipeline functions

### No Changes to `op.rs`

**Important note**: The core `Op` trait in `op.rs` was **not modified**. This is a key design principle:

**Why no changes to Op:**
- **Backward compatibility**: All existing code continues to work
- **Non-invasive design**: New functionality doesn't break existing patterns
- **Extensibility**: The `Op` trait is flexible enough to support new implementations

**How it works without changes:**
- Our new `StreamingPrompt` struct implements the existing `Op` trait
- The trait's design allows for different input/output types via generics
- No modification needed because `Op` was already designed to be extensible

### Memory and Performance Characteristics

**For novice developers - what actually happens at runtime:**

1. **Zero-cost abstractions:**
   - `PhantomData`: Completely disappears at compile time
   - Generic types: Compile to specific types (monomorphization)
   - **Result**: No runtime overhead from our abstractions

2. **Stream processing:**
   - **Lazy evaluation**: Chunks only generated when you request them
   - **Backpressure**: If you stop reading, generation pauses
   - **Memory efficient**: Only one chunk in memory at a time

3. **Error handling:**
   - **Fail-fast**: If streaming setup fails, you get an immediate error
   - **Graceful degradation**: Individual chunk errors don't stop the stream

### Design Philosophy: Why This Approach?

**For novice developers - the "why" behind the design:**

1. **Separation of concerns:**
   - **Rig handles**: Stream generation and low-level streaming protocols
   - **Your code handles**: Data structuring, sending, processing logic

2. **Type safety:**
   - **Compile-time guarantees**: Can't accidentally use non-streaming models
   - **Generic flexibility**: Works with any string-like input, any streaming model

3. **Async-first design:**
   - **Non-blocking**: Streaming doesn't block other operations
   - **Concurrent**: Can process multiple streams simultaneously

4. **Rust idioms:**
   - **Zero-cost abstractions**: Performance with safety
   - **Ownership model**: Clear ownership of data and streams
   - **Error handling**: Explicit error types, no hidden failures

This implementation showcases many core Rust concepts: generics, traits, async programming, error handling, and zero-cost abstractions - all working together to provide a safe, fast, and flexible streaming interface!

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