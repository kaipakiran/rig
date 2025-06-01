/// Example showing how to convert from traditional .map().prompt() pattern to streaming
/// 
/// This demonstrates the `streaming_prompt_with_sender` functionality which allows you to:
/// 1. Get real-time streaming updates via tokio channels
/// 2. Still use the regular Op interface and get a final aggregated response
/// 3. Process streaming chunks in a separate task while the main task continues
///
/// This is perfect for building real-time chat interfaces or progress indicators.

use rig::{
    pipeline::{self, Op, streaming_prompt_with_sender},
    providers::gemini::{self, completion::GEMINI_1_5_FLASH},
};
use tokio::sync::mpsc;
use std::io::Write;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = gemini::Client::from_env();
    let input_text = "a robot learning to paint";

    println!("=== Traditional Pipeline Pattern (Inline Agent Build) ===");
    
    // TRADITIONAL PATTERN: map -> prompt with inline agent.build()
    let traditional_pipeline = pipeline::new()
        .map(|input: String| format!("Write a short story about: {}", input))
        .prompt(
            client.agent(GEMINI_1_5_FLASH)
                .preamble("You are a creative storyteller. Write engaging short stories.")
                .temperature(0.7)
                .build()
        );
    
    println!("Processing with traditional pipeline...");
    let traditional_result = traditional_pipeline.call(input_text.to_string()).await?;
    println!("Traditional result length: {} characters", traditional_result.len());
    println!("First 100 chars: {}\n", &traditional_result[..traditional_result.len().min(100)]);

    println!("=== Streaming Pipeline Pattern (Inline Agent Build) ===");

    // Create a tokio channel for streaming chunks
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Spawn a task to handle real-time streaming output
    let _stream_handler = tokio::spawn(async move {
        print!("Streaming response: ");
        std::io::stdout().flush().unwrap();
        
        // Display chunks as they arrive
        while let Some(chunk) = rx.recv().await {
            print!("{}", chunk);
            std::io::stdout().flush().unwrap();
        }
        println!(); // New line after streaming
    });

    // Create sender closure
    let sender_closure = {
        let tx_clone = tx.clone();
        move |chunk: String| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            tx_clone.send(chunk).map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        }
    };

    // STREAMING PATTERN: map -> streaming_prompt_with_sender with inline agent.build()
    let streaming_pipeline = pipeline::new()
        .map(|input: String| format!("Write a short story about: {}", input))
        .chain(streaming_prompt_with_sender(
            client.agent(GEMINI_1_5_FLASH)
                .preamble("You are a creative storyteller. Write engaging short stories.")
                .temperature(0.7)
                .build(),
            sender_closure
        ));

    println!("Processing with streaming pipeline...");
    let streaming_result = streaming_pipeline.call(input_text.to_string()).await?;
    
    // Give streaming a moment to complete display
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    
    println!("\nStreaming result length: {} characters", streaming_result.len());
    println!("First 100 chars: {}", &streaming_result[..streaming_result.len().min(100)]);

    println!("\n=== Alternative: Using PipelineBuilder Method ===");

    // You can also use the method form if you build the agent first in the pipeline
    let (tx2, mut rx2) = mpsc::unbounded_channel::<String>();
    
    let _stream_handler2 = tokio::spawn(async move {
        while let Some(chunk) = rx2.recv().await {
            print!("{}", chunk);
            std::io::stdout().flush().unwrap();
        }
        println!();
    });

    let sender_closure2 = {
        let tx_clone = tx2.clone();
        move |chunk: String| -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            tx_clone.send(chunk).map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        }
    };

    // Using the PipelineBuilder method (can't inline agent.build() here)
    let agent = client.agent(GEMINI_1_5_FLASH)
        .preamble("You are a creative storyteller. Write engaging short stories.")
        .temperature(0.7)
        .build();

    let alternative_pipeline = pipeline::new()
        .streaming_prompt_with_sender(agent, sender_closure2)
        .map(|result| match result {
            Ok(text) => format!("Story complete: {} characters", text.len()),
            Err(e) => format!("Error: {}", e),
        });

    println!("Processing with PipelineBuilder method...");
    let alternative_result = alternative_pipeline.call("Write a short story about: a robot learning to paint".to_string()).await;
    
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    println!("\nAlternative result: {}", alternative_result);

    println!("\nDone!");
    Ok(())
} 