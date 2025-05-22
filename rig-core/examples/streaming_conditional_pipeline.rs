use rig::{
    pipeline::{self, Op, model_streaming, streaming_pipeline},
    providers::gemini::{self, completion::GEMINI_1_5_FLASH},
    message::{AssistantContent, Message},
    completion::Prompt,
    conditional,
};
use futures::StreamExt;
use std::io::Write;
use tokio;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create client and model
    let client = gemini::Client::from_env();
    let model = client.completion_model(GEMINI_1_5_FLASH);
    
    // Create a streaming model adapter
    let streaming_model = model_streaming(model.clone());
    
    // Create a classifier agent
    let classifier = client.agent(GEMINI_1_5_FLASH)
        .preamble("You are a query classifier. Respond with one word: 'weather' if the query is about weather, or 'general' for anything else.")
        .build();
    
    println!("=== Streaming Conditional Pipeline ===");
    println!("This pipeline classifies your query and routes it to the appropriate expert with streaming responses.");
    println!("Try asking about weather in a location or ask a general question.\n");
    
    // Get user query
    print!("Enter your query: ");
    std::io::stdout().flush()?;
    let mut query = String::new();
    std::io::stdin().read_line(&mut query)?;
    let query = query.trim().to_string();
    
    // Step 1: Classify the query
    let classify_prompt = format!("Classify this query as either 'weather' or 'general': '{}'", query);
    let classification = classifier.prompt(&classify_prompt).await?;
    let is_weather = classification.to_lowercase().contains("weather");
    
    println!("\nClassified as: {}", if is_weather { "Weather Query" } else { "General Query" });
    
    // Step 2: Prepare the prompt based on classification
    let prompt = if is_weather {
        println!("Routing to weather specialist with streaming response...\n");
        format!("You are a weather specialist. Provide weather information for: {}", query)
    } else {
        println!("Routing to general assistant with streaming response...\n");
        format!("You are a helpful assistant. Answer this query: {}", query)
    };
    
    // Step 3: Create a streaming pipeline with the appropriate prompt
    let pipeline = streaming_pipeline(streaming_model)
        .map(|content| {
            match content {
                AssistantContent::Text(text) => text.text,
                AssistantContent::ToolCall(tool) => format!("\nTool call: {}\n", tool.function.name),
            }
        });
    
    // Step 4: Stream the response
    let mut stream = pipeline.stream(prompt).await;
    
    print!("Response: ");
    std::io::stdout().flush()?;
    
    while let Some(text) = stream.next().await {
        print!("{}", text);
        std::io::stdout().flush()?;
    }
    
    println!("\n\nDone!");
    Ok(())
} 