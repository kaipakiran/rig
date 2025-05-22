use rig::{
    pipeline::{self, model_streaming, streaming_pipeline},
    providers::gemini::{self, completion::GEMINI_1_5_FLASH},
    completion::Prompt,
    message::AssistantContent,
};
use std::io::Write;
use tokio;
use std::sync::Arc;
use futures::StreamExt;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client
    let client = gemini::Client::from_env();
    
    // Create classifier agent
    let classifier = Arc::new(client.agent(GEMINI_1_5_FLASH)
        .preamble("You are a query classifier. Respond with one word: 'weather' if the query is about weather, or 'general' for anything else.")
        .build());
    
    // Create model for streaming responses
    let model = client.completion_model(GEMINI_1_5_FLASH);
    let streaming_model = model_streaming(model.clone());
    
    println!("=== Functional Conditional Streaming Pipeline ===");
    println!("This pipeline demonstrates conditional branching with streaming responses.");
    println!("Try asking about weather in a location or ask a general question.\n");
    
    // Prompt for user input
    print!("Enter your query: ");
    std::io::stdout().flush()?;
    let mut query = String::new();
    std::io::stdin().read_line(&mut query)?;
    let query = query.trim().to_string();
    
    println!("\nProcessing your query...");
    
    // First classify the query using the classifier agent
    let classification_prompt = format!(
        "Classify this query as either 'weather' or 'general': '{}'", 
        query
    );
    
    let classification = classifier
        .prompt(&classification_prompt)
        .await
        .unwrap_or_else(|_| "general".to_string());
    
    let is_weather = classification.to_lowercase().contains("weather");
    println!("\nClassified as: {}", if is_weather { "Weather Query" } else { "General Query" });
    
    // Create a streaming pipeline
    let text_pipeline = streaming_pipeline(streaming_model)
        .map(|content| match content {
            AssistantContent::Text(text) => text.text,
            AssistantContent::ToolCall(tool) => format!("Tool call: {}\n", tool.function.name),
        });
    
    // Based on classification, adjust the prompt and stream response
    let final_query = if is_weather {
        println!("\nStreaming weather information...");
        format!("Provide detailed weather information for: {}", query)
    } else {
        println!("\nStreaming general information...");
        query
    };
    
    println!("\nResponse:");
    
    // Create the appropriate stream
    let mut stream = text_pipeline.stream(final_query).await;
    
    // Process the streaming response
    while let Some(chunk) = stream.next().await {
        print!("{}", chunk);
        std::io::stdout().flush()?;
    }
    
    println!("\n\nDone!");
    Ok(())
} 