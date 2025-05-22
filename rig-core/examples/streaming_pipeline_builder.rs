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
    // Create a client and model adapter
    let client = gemini::Client::from_env();
    let model = model_streaming(client.completion_model(GEMINI_1_5_FLASH));
    
    // Create a streaming pipeline with the model using PipelineBuilder
    let pipeline = pipeline::new()
        .streaming_prompt(model)
        .map(|content| {
            match content {
                AssistantContent::Text(text) => text.text,
                AssistantContent::ToolCall(tool) => format!("\nTool call: {}\n", tool.function.name),
            }
        });

    // Run the pipeline with an input
    println!("Starting streaming pipeline with builder...\n");
    let mut stream = pipeline.stream("Write a short story about a robot learning to paint").await;

    // Process the streaming response
    while let Some(text) = stream.next().await {
        print!("{}", text);
        std::io::stdout().flush()?;
    }

    println!("\n\nDone!");
    Ok(())
} 