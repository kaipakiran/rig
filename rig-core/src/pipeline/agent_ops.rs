use std::future::IntoFuture;

use crate::{
    completion::{self, CompletionModel, CompletionError},
    extractor::{ExtractionError, Extractor},
    message::Message,
    vector_store,
    streaming::{StreamingCompletionModel, StreamingPrompt as StreamingPromptTrait},
    agent::Agent,
};

use super::op::Op;
use futures::StreamExt;

/// Trait for sending streaming chunks
pub trait StreamingSender: Send + Sync {
    /// Send a chunk of text
    fn send_chunk(&self, chunk: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    
    /// Signal that streaming is complete (optional - default implementation does nothing)
    fn close(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Ok(())
    }
}

/// Implementation for any closure that sends strings
impl<F> StreamingSender for F 
where
    F: Fn(String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> + Send + Sync,
{
    fn send_chunk(&self, chunk: String) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        self(chunk)
    }
}

pub struct Lookup<I, In, T> {
    index: I,
    n: usize,
    _in: std::marker::PhantomData<In>,
    _t: std::marker::PhantomData<T>,
}

impl<I, In, T> Lookup<I, In, T>
where
    I: vector_store::VectorStoreIndex,
{
    pub(crate) fn new(index: I, n: usize) -> Self {
        Self {
            index,
            n,
            _in: std::marker::PhantomData,
            _t: std::marker::PhantomData,
        }
    }
}

impl<I, In, T> Op for Lookup<I, In, T>
where
    I: vector_store::VectorStoreIndex,
    In: Into<String> + Send + Sync,
    T: Send + Sync + for<'a> serde::Deserialize<'a>,
{
    type Input = In;
    type Output = Result<Vec<(f64, String, T)>, vector_store::VectorStoreError>;

    async fn call(&self, input: Self::Input) -> Self::Output {
        let query: String = input.into();

        let docs = self
            .index
            .top_n::<T>(&query, self.n)
            .await?
            .into_iter()
            .collect();

        Ok(docs)
    }
}

/// Create a new lookup operation.
///
/// The op will perform semantic search on the provided index and return the top `n`
/// results closest results to the input.
pub fn lookup<I, In, T>(index: I, n: usize) -> Lookup<I, In, T>
where
    I: vector_store::VectorStoreIndex,
    In: Into<String> + Send + Sync,
    T: Send + Sync + for<'a> serde::Deserialize<'a>,
{
    Lookup::new(index, n)
}

pub struct Prompt<P, In> {
    prompt: P,
    _in: std::marker::PhantomData<In>,
}

impl<P, In> Prompt<P, In> {
    pub(crate) fn new(prompt: P) -> Self {
        Self {
            prompt,
            _in: std::marker::PhantomData,
        }
    }
}

impl<P, In> Op for Prompt<P, In>
where
    P: completion::Prompt + Send + Sync,
    In: Into<String> + Send + Sync,
{
    type Input = In;
    type Output = Result<String, completion::PromptError>;

    fn call(&self, input: Self::Input) -> impl std::future::Future<Output = Self::Output> + Send {
        self.prompt.prompt(input.into()).into_future()
    }
}

/// Create a new prompt operation.
///
/// The op will prompt the `model` with the input and return the response.
pub fn prompt<P, In>(model: P) -> Prompt<P, In>
where
    P: completion::Prompt,
    In: Into<String> + Send + Sync,
{
    Prompt::new(model)
}

pub struct Extract<M, Input, Output>
where
    M: CompletionModel,
    Output: schemars::JsonSchema + for<'a> serde::Deserialize<'a> + Send + Sync,
{
    extractor: Extractor<M, Output>,
    _in: std::marker::PhantomData<Input>,
}

impl<M, Input, Output> Extract<M, Input, Output>
where
    M: CompletionModel,
    Output: schemars::JsonSchema + for<'a> serde::Deserialize<'a> + Send + Sync,
{
    pub(crate) fn new(extractor: Extractor<M, Output>) -> Self {
        Self {
            extractor,
            _in: std::marker::PhantomData,
        }
    }
}

impl<M, Input, Output> Op for Extract<M, Input, Output>
where
    M: CompletionModel,
    Output: schemars::JsonSchema + for<'a> serde::Deserialize<'a> + Send + Sync,
    Input: Into<Message> + Send + Sync,
{
    type Input = Input;
    type Output = Result<Output, ExtractionError>;

    async fn call(&self, input: Self::Input) -> Self::Output {
        self.extractor.extract(input).await
    }
}

/// Create a new extract operation.
///
/// The op will extract the structured data from the input using the provided `extractor`.
pub fn extract<M, Input, Output>(extractor: Extractor<M, Output>) -> Extract<M, Input, Output>
where
    M: CompletionModel,
    Output: schemars::JsonSchema + for<'a> serde::Deserialize<'a> + Send + Sync,
    Input: Into<String> + Send + Sync,
{
    Extract::new(extractor)
}

pub struct StreamingPromptWithSender<P, In, S> {
    prompt: P,
    sender: S,
    _in: std::marker::PhantomData<In>,
}

impl<P, In, S> StreamingPromptWithSender<P, In, S>
where
    P: Send + Sync,
    In: Send + Sync,
    S: StreamingSender,
{
    pub fn new(prompt: P, sender: S) -> Self {
        Self {
            prompt,
            sender,
            _in: std::marker::PhantomData,
        }
    }
}

// Implementation for Agent types that implement StreamingPrompt
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
        let message = Message::user(input.into());
        let mut stream = self.prompt.stream_prompt(message).await?;
        
        let mut final_text = String::new();
        
        // Stream the chunks and send them via the sender
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(crate::message::AssistantContent::Text(text)) => {
                    // Send the chunk via the sender (ignore send errors)
                    let _ = self.sender.send_chunk(text.text.clone());
                    // Accumulate for final response
                    final_text.push_str(&text.text);
                }
                Ok(crate::message::AssistantContent::ToolCall(_)) => {
                    // Handle tool calls if needed, for now just continue
                    continue;
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
        
        // Signal that streaming is complete
        let _ = self.sender.close();
        
        Ok(final_text)
    }
}

/// Create a new streaming prompt operation that sends chunks via a generic sender and returns the final response.
///
/// The op will stream the response from the `model`, send each text chunk via the provided `sender`,
/// and return the final aggregated response as a string.
pub fn streaming_prompt_with_sender<P, In, S>(
    model: P, 
    sender: S
) -> StreamingPromptWithSender<P, In, S>
where
    P: Send + Sync,
    In: Send + Sync,
    S: StreamingSender,
{
    StreamingPromptWithSender::new(model, sender)
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::message;
    use completion::{Prompt, PromptError};
    use vector_store::{VectorStoreError, VectorStoreIndex};

    pub struct MockModel;

    impl Prompt for MockModel {
        #[allow(refining_impl_trait)]
        async fn prompt(&self, prompt: impl Into<message::Message>) -> Result<String, PromptError> {
            let msg: message::Message = prompt.into();
            let prompt = match msg {
                message::Message::User { content } => match content.first() {
                    message::UserContent::Text(message::Text { text }) => text,
                    _ => unreachable!(),
                },
                _ => unreachable!(),
            };
            Ok(format!("Mock response: {}", prompt))
        }
    }

    pub struct MockIndex;

    impl VectorStoreIndex for MockIndex {
        async fn top_n<T: for<'a> serde::Deserialize<'a> + std::marker::Send>(
            &self,
            _query: &str,
            _n: usize,
        ) -> Result<Vec<(f64, String, T)>, VectorStoreError> {
            let doc = serde_json::from_value(serde_json::json!({
                "foo": "bar",
            }))
            .unwrap();

            Ok(vec![(1.0, "doc1".to_string(), doc)])
        }

        async fn top_n_ids(
            &self,
            _query: &str,
            _n: usize,
        ) -> Result<Vec<(f64, String)>, VectorStoreError> {
            Ok(vec![(1.0, "doc1".to_string())])
        }
    }

    #[derive(Debug, serde::Deserialize, PartialEq)]
    pub struct Foo {
        pub foo: String,
    }

    #[tokio::test]
    async fn test_lookup() {
        let index = MockIndex;
        let lookup = lookup::<MockIndex, String, Foo>(index, 1);

        let result = lookup.call("query".to_string()).await.unwrap();
        assert_eq!(
            result,
            vec![(
                1.0,
                "doc1".to_string(),
                Foo {
                    foo: "bar".to_string()
                }
            )]
        );
    }

    #[tokio::test]
    async fn test_prompt() {
        let model = MockModel;
        let prompt = prompt::<MockModel, String>(model);

        let result = prompt.call("hello".to_string()).await.unwrap();
        assert_eq!(result, "Mock response: hello");
    }
}
