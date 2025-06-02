/// Example showing external streaming processing with custom structured output
use rig::{
    pipeline::{self, streaming_prompt, Op},
    providers::gemini::{self, completion::GEMINI_1_5_FLASH},
    message::AssistantContent,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use std::io::Write;

// Your custom structured output format
#[derive(Debug, Serialize, Deserialize)]
struct MyStructData {
    timestamp: u64,
    content: String,
    chunk_id: usize,
    word_count: usize,
    has_punctuation: bool,
    estimated_reading_time_ms: u64,
    // Add any other fields you need
    session_id: String,
    user_id: Option<String>,
}

async fn process_streaming_externally() -> Result<(), Box<dyn std::error::Error>> {
    let client = gemini::Client::from_env();
    let agent = client.agent(GEMINI_1_5_FLASH)
        .preamble("You are a helpful assistant that provides detailed explanations.")
        .build();

    // Create your direct channel for your custom struct
    let (tx, mut rx) = mpsc::unbounded_channel::<MyStructData>();

    // Create pipeline that returns the stream for external processing
    let pipeline = pipeline::new()
        .map(|input: String| format!("Explain in detail: {}", input))
        .chain(streaming_prompt(agent));

    // Spawn task to handle your custom structured data
    tokio::spawn(async move {
        while let Some(my_data) = rx.recv().await {
            // Your custom processing - format as JSON, save to DB, send to websocket, etc.
            println!("📦 Received: Chunk #{} - {} words, session: {}", 
                my_data.chunk_id, 
                my_data.word_count,
                my_data.session_id
            );
            
            // Example: Send to multiple destinations
            send_to_websocket(&my_data).await.ok();
            save_to_database(&my_data).await.ok();
            update_metrics(&my_data).await.ok();
        }
    });

    // Execute the pipeline to get the streaming response
    let stream_response = pipeline.call("quantum computing and its applications".to_string()).await?;

    // Now YOU handle the streaming and processing externally
    let mut stream = stream_response;
    let mut chunk_id = 0;
    let session_id = format!("session_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs());
    
    println!("🚀 Starting external streaming processing...\n");
    
    while let Some(chunk_result) = stream.next().await {
        match chunk_result {
            Ok(AssistantContent::Text(text)) => {
                chunk_id += 1;
                
                // Create YOUR custom structured data
                let my_struct_data = MyStructData {
                    timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_millis() as u64,
                    content: text.text.clone(),
                    chunk_id,
                    word_count: text.text.split_whitespace().count(),
                    has_punctuation: text.text.chars().any(|c| c.is_ascii_punctuation()),
                    estimated_reading_time_ms: (text.text.len() as u64 * 5), // ~200 WPM
                    session_id: session_id.clone(),
                    user_id: Some("user123".to_string()), // Your user tracking
                };

                // Direct send of YOUR custom struct
                if let Err(e) = tx.send(my_struct_data) {
                    eprintln!("Failed to send custom struct: {}", e);
                }

                // Also display real-time (optional)
                print!("{}", text.text);
                std::io::stdout().flush()?;
            }
            Ok(AssistantContent::ToolCall(tool_call)) => {
                println!("\n🔧 Tool call: {}", tool_call.function.name);
            }
            Err(e) => {
                eprintln!("❌ Streaming error: {}", e);
                break;
            }
        }
    }

    println!("\n\n✅ Streaming complete!");
    println!("📊 Total chunks processed: {}", chunk_id);
    
    // Small delay to let background processing finish
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    Ok(())
}

// Mock functions for external integrations (using your custom struct)
async fn send_to_websocket(data: &MyStructData) -> Result<(), Box<dyn std::error::Error>> {
    // Your WebSocket sending logic with your custom struct
    let json_data = serde_json::to_string(data)?;
    println!("🌐 Sent to WebSocket: {}", json_data);
    Ok(())
}

async fn save_to_database(data: &MyStructData) -> Result<(), Box<dyn std::error::Error>> {
    // Your database saving logic with your custom struct
    println!("💾 Saved to DB: chunk #{}, session: {}", data.chunk_id, data.session_id);
    Ok(())
}

async fn update_metrics(data: &MyStructData) -> Result<(), Box<dyn std::error::Error>> {
    // Your metrics/analytics logic with your custom struct
    println!("📈 Updated metrics: {} words at {}", data.word_count, data.timestamp);
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== External Streaming Processing Example ===");
    println!("This example shows how to use tx.send(MyStructData) for custom structured output.\n");
    
    process_streaming_externally().await?;
    
    Ok(())
} 