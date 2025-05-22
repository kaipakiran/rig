use rig::{
    pipeline::{self, model_streaming, streaming_pipeline, StreamingPipelineStage},
    providers::gemini::{self, completion::GEMINI_1_5_FLASH},
    message::AssistantContent,
};
use futures::StreamExt;
use std::io::Write;
use std::pin::Pin;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a client and model
    let client = gemini::Client::from_env();
    let model = client.completion_model(GEMINI_1_5_FLASH);
    
    // Create a streaming pipeline with the model
    let pipeline = streaming_pipeline(model_streaming(model))
        .map(|content| {
            match content {
                AssistantContent::Text(text) => text.text,
                AssistantContent::ToolCall(tool) => format!("\nTool call: {}\n", tool.function.name),
            }
        });

    // Run the pipeline with an input
    println!("Starting streaming pipeline...\n");
    let mut stream = pipeline.stream("Write a short story about a robot learning to paint").await;

    // Process the streaming response
    while let Some(text) = stream.next().await {
        print!("{}", text);
        std::io::stdout().flush()?;
    }

    println!("\n\nDone!");
    Ok(())
} 