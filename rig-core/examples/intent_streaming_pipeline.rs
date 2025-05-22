use rig::{
    pipeline::{self, model_streaming, streaming_pipeline, StreamingPipelineStage, Op},
    providers::gemini::{self, completion::GEMINI_1_5_FLASH},
    message::{AssistantContent, Message},
    completion::{CompletionError, Prompt},
    agent::{Agent, AgentBuilder},
};
use futures::{Stream, StreamExt};
use std::io::Write;
use std::pin::Pin;
use tokio;
use serde::{Deserialize, Serialize};

/// Intent detection response structure
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
struct IntentDetection {
    /// The detected intent type
    intent: String,
    /// Confidence score (0-1)
    confidence: f32,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a client
    let client = gemini::Client::from_env();
    let model = client.completion_model(GEMINI_1_5_FLASH);
    
    println!("Starting intent-based streaming pipeline...\n");
    
    // Process user query
    let user_query = prompt_user("Enter your query: ")?;
    
    // Create intent detector with structured output
    let intent_extractor = client.extractor::<IntentDetection>("gemini-1.5-flash")
        .preamble("You are an intent detection specialist. Analyze queries to determine if they're about weather or general information.")
        .build();
    
    // First extract the intent
    let intent_result = intent_extractor.extract(&user_query).await?;
    
    println!("Detected intent: {} (confidence: {:.2})", 
             intent_result.intent, 
             intent_result.confidence);
    
    // Create a streaming adapter for the model
    let streaming_model = model_streaming(model.clone());
    
    // Based on intent, choose the appropriate pipeline
    if intent_result.intent.to_lowercase() == "weather" {
        // For weather intent, use simulated weather search
        println!("\nUsing simulated weather search for query...\n");
        
        // Create a pipeline that enhances the weather query with simulated search results
        let pipeline = pipeline::new()
            .map(move |_| {
                format!("I need information about the current weather in {}", user_query)
            })
            // Note: Using Op::then instead of StreamExt::then
            .then(|query: String| async move {
                let search_results = simulated_weather_search(&query).await
                    .unwrap_or_else(|e| format!("Error searching: {}", e));
                
                // Return the results directly
                format!("Weather information:\n{}", search_results)
            });
        
        let result = pipeline.call(()).await;
        println!("{}", result);
    } else {
        // For general intent, use streaming response
        println!("\nUsing streaming chat for general query...\n");
        
        // Create a streaming pipeline with the model
        let pipeline = streaming_pipeline(streaming_model)
            .map(|content| {
                match content {
                    AssistantContent::Text(text) => text.text,
                    AssistantContent::ToolCall(tool) => format!("\nTool call: {}\n", tool.function.name),
                }
            });
        
        // Stream the response
        let mut stream = pipeline.stream(&user_query).await;
        
        print!("Response: ");
        while let Some(text) = stream.next().await {
            print!("{}", text);
            std::io::stdout().flush()?;
        }
        println!();
    }
    
    println!("\nDone!");
    Ok(())
}

/// Simulated weather search function
async fn simulated_weather_search(query: &str) -> Result<String, Box<dyn std::error::Error>> {
    // Simulate a delay
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
    
    // Return different responses based on location mentions in the query
    let response = if query.to_lowercase().contains("london") {
        "London: Currently 15°C with light rain. Forecast: Rain expected to continue throughout the day with temperatures between 13-17°C."
    } else if query.to_lowercase().contains("new york") {
        "New York: Currently 22°C and sunny. Forecast: Clear skies with temperatures between 20-25°C."
    } else if query.to_lowercase().contains("tokyo") {
        "Tokyo: Currently 28°C and humid. Forecast: Partly cloudy with a chance of evening showers."
    } else {
        "Weather information not found for the specified location. Please try again with a specific city name."
    };
    
    Ok(response.to_string())
}

/// Prompt the user for input
fn prompt_user(prompt: &str) -> Result<String, Box<dyn std::error::Error>> {
    print!("{}", prompt);
    std::io::stdout().flush()?;
    
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    
    Ok(input.trim().to_string())
} 