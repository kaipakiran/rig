//! This module defines a flexible pipeline API for defining a sequence of operations that
//! may or may not use AI components (e.g.: semantic search, LLMs prompting, etc).
//!
//! The pipeline API was inspired by general orchestration pipelines such as Airflow, Dagster and Prefect,
//! but implemented with idiomatic Rust patterns and providing some AI-specific ops out-of-the-box along
//! general combinators.
//!
//! Pipelines are made up of one or more operations, or "ops", each of which must implement the [Op] trait.
//! The [Op] trait requires the implementation of only one method: `call`, which takes an input
//! and returns an output. The trait provides a wide range of combinators for chaining operations together.
//!
//! One can think of a pipeline as a DAG (Directed Acyclic Graph) where each node is an operation and
//! the edges represent the data flow between operations. When invoking the pipeline on some input,
//! the input is passed to the root node of the DAG (i.e.: the first op defined in the pipeline).
//! The output of each op is then passed to the next op in the pipeline until the output reaches the
//! leaf node (i.e.: the last op defined in the pipeline). The output of the leaf node is then returned
//! as the result of the pipeline.
//!
//! ## Basic Example
//! For example, the pipeline below takes a tuple of two integers, adds them together and then formats
//! the result as a string using the [map](Op::map) combinator method, which applies a simple function
//! op to the output of the previous op:
//! ```rust
//! use rig::pipeline::{self, Op};
//!
//! let pipeline = pipeline::new()
//!     // op1: add two numbers
//!     .map(|(x, y)| x + y)
//!     // op2: format result
//!     .map(|z| format!("Result: {z}!"));
//!
//! let result = pipeline.call((1, 2)).await;
//! assert_eq!(result, "Result: 3!");
//! ```
//!
//! This pipeline can be visualized as the following DAG:
//! ```text
//!          ┌─────────┐   ┌─────────┐         
//! Input───►│   op1   ├──►│   op2   ├──►Output
//!          └─────────┘   └─────────┘         
//! ```
//!
//! ## Parallel Operations
//! The pipeline API also provides a [parallel!](crate::parallel!) and macro for running operations in parallel.
//! The macro takes a list of ops and turns them into a single op that will duplicate the input
//! and run each op in concurrently. The results of each op are then collected and returned as a tuple.
//!
//! For example, the pipeline below runs two operations concurrently:
//! ```rust
//! use rig::{pipeline::{self, Op, map}, parallel};
//!
//! let pipeline = pipeline::new()
//!     .chain(parallel!(
//!         // op1: add 1 to input
//!         map(|x| x + 1),
//!         // op2: subtract 1 from input
//!         map(|x| x - 1),
//!     ))
//!     // op3: format results
//!     .map(|(a, b)| format!("Results: {a}, {b}"));
//!
//! let result = pipeline.call(1).await;
//! assert_eq!(result, "Result: 2, 0");
//! ```
//!
//! Notes:
//! - The [chain](Op::chain) method is similar to the [map](Op::map) method but it allows
//!   for chaining arbitrary operations, as long as they implement the [Op] trait.
//! - [map] is a function that initializes a standalone [Map](self::op::Map) op without an existing pipeline/op.
//!
//! The pipeline above can be visualized as the following DAG:
//! ```text                 
//!           Input            
//!             │              
//!      ┌──────┴──────┐       
//!      ▼             ▼       
//! ┌─────────┐   ┌─────────┐  
//! │   op1   │   │   op2   │  
//! └────┬────┘   └────┬────┘  
//!      └──────┬──────┘       
//!             ▼              
//!        ┌─────────┐         
//!        │   op3   │         
//!        └────┬────┘         
//!             │              
//!             ▼              
//!          Output           
//! ```

pub mod agent_ops;
pub mod op;
pub mod try_op;
#[macro_use]
pub mod parallel;
#[macro_use]
pub mod conditional;

use std::future::Future;
use futures::{Stream, StreamExt};
use std::marker::PhantomData;
use std::pin::Pin;

pub use op::{map, passthrough, then, Op, StreamingOp, StreamingMap, StreamingThen, StreamingPassthrough};
pub use try_op::TryOp;
pub use agent_ops::{StreamingPrompt, StreamingSender};

use crate::{completion, extractor::Extractor, vector_store, streaming::{StreamingPrompt as StreamingPromptTrait, StreamingCompletionResponse, StreamingCompletionModel}, message::{Message, AssistantContent}, completion::CompletionError};

pub struct PipelineBuilder<E> {
    _error: std::marker::PhantomData<E>,
}

impl<E> PipelineBuilder<E> {
    /// Add a function to the current pipeline
    ///
    /// # Example
    /// ```rust
    /// use rig::pipeline::{self, Op};
    ///
    /// let pipeline = pipeline::new()
    ///    .map(|(x, y)| x + y)
    ///    .map(|z| format!("Result: {z}!"));
    ///
    /// let result = pipeline.call((1, 2)).await;
    /// assert_eq!(result, "Result: 3!");
    /// ```
    pub fn map<F, Input, Output>(self, f: F) -> op::Map<F, Input>
    where
        F: Fn(Input) -> Output + Send + Sync,
        Input: Send + Sync,
        Output: Send + Sync,
        Self: Sized,
    {
        op::Map::new(f)
    }

    /// Same as `map` but for asynchronous functions
    ///
    /// # Example
    /// ```rust
    /// use rig::pipeline::{self, Op};
    ///
    /// let pipeline = pipeline::new()
    ///     .then(|email: String| async move {
    ///         email.split('@').next().unwrap().to_string()
    ///     })
    ///     .then(|username: String| async move {
    ///         format!("Hello, {}!", username)
    ///     });
    ///
    /// let result = pipeline.call("bob@gmail.com".to_string()).await;
    /// assert_eq!(result, "Hello, bob!");
    /// ```
    pub fn then<F, Input, Fut>(self, f: F) -> op::Then<F, Input>
    where
        F: Fn(Input) -> Fut + Send + Sync,
        Input: Send + Sync,
        Fut: Future + Send + Sync,
        Fut::Output: Send + Sync,
        Self: Sized,
    {
        op::Then::new(f)
    }

    /// Add an arbitrary operation to the current pipeline.
    ///
    /// # Example
    /// ```rust
    /// use rig::pipeline::{self, Op};
    ///
    /// struct MyOp;
    ///
    /// impl Op for MyOp {
    ///     type Input = i32;
    ///     type Output = i32;
    ///
    ///     async fn call(&self, input: Self::Input) -> Self::Output {
    ///         input + 1
    ///     }
    /// }
    ///
    /// let pipeline = pipeline::new()
    ///    .chain(MyOp);
    ///
    /// let result = pipeline.call(1).await;
    /// assert_eq!(result, 2);
    /// ```
    pub fn chain<T>(self, op: T) -> T
    where
        T: Op,
        Self: Sized,
    {
        op
    }

    /// Chain a lookup operation to the current chain. The lookup operation expects the
    /// current chain to output a query string. The lookup operation will use the query to
    /// retrieve the top `n` documents from the index and return them with the query string.
    ///
    /// # Example
    /// ```rust
    /// use rig::pipeline::{self, Op};
    ///
    /// let pipeline = pipeline::new()
    ///     .lookup(index, 2)
    ///     .pipeline(|(query, docs): (_, Vec<String>)| async move {
    ///         format!("User query: {}\n\nTop documents:\n{}", query, docs.join("\n"))
    ///     });
    ///
    /// let result = pipeline.call("What is a flurbo?".to_string()).await;
    /// ```
    pub fn lookup<I, Input, Output>(self, index: I, n: usize) -> agent_ops::Lookup<I, Input, Output>
    where
        I: vector_store::VectorStoreIndex,
        Output: Send + Sync + for<'a> serde::Deserialize<'a>,
        Input: Into<String> + Send + Sync,
        // E: From<vector_store::VectorStoreError> + Send + Sync,
        Self: Sized,
    {
        agent_ops::Lookup::new(index, n)
    }

    /// Add a prompt operation to the current pipeline/op. The prompt operation expects the
    /// current pipeline to output a string. The prompt operation will use the string to prompt
    /// the given `agent`, which must implements the [Prompt](completion::Prompt) trait and return
    /// the response.
    ///
    /// # Example
    /// ```rust
    /// use rig::pipeline::{self, Op};
    ///
    /// let agent = &openai_client.agent("gpt-4").build();
    ///
    /// let pipeline = pipeline::new()
    ///    .map(|name| format!("Find funny nicknames for the following name: {name}!"))
    ///    .prompt(agent);
    ///
    /// let result = pipeline.call("Alice".to_string()).await;
    /// ```
    pub fn prompt<P, Input>(self, agent: P) -> agent_ops::Prompt<P, Input>
    where
        P: completion::Prompt,
        Input: Into<String> + Send + Sync,
        // E: From<completion::PromptError> + Send + Sync,
        Self: Sized,
    {
        agent_ops::Prompt::new(agent)
    }

    /// Add an extract operation to the current pipeline/op. The extract operation expects the
    /// current pipeline to output a string. The extract operation will use the given `extractor`
    /// to extract information from the string in the form of the type `T` and return it.
    ///
    /// # Example
    /// ```rust
    /// use rig::pipeline::{self, Op};
    ///
    /// #[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
    /// struct Sentiment {
    ///     /// The sentiment score of the text (0.0 = negative, 1.0 = positive)
    ///     score: f64,
    /// }
    ///
    /// let extractor = &openai_client.extractor::<Sentiment>("gpt-4").build();
    ///
    /// let pipeline = pipeline::new()
    ///     .map(|text| format!("Analyze the sentiment of the following text: {text}!"))
    ///     .extract(extractor);
    ///
    /// let result: Sentiment = pipeline.call("I love ice cream!".to_string()).await?;
    /// assert!(result.score > 0.5);
    /// ```
    pub fn extract<M, Input, Output>(
        self,
        extractor: Extractor<M, Output>,
    ) -> agent_ops::Extract<M, Input, Output>
    where
        M: completion::CompletionModel,
        Output: schemars::JsonSchema + for<'a> serde::Deserialize<'a> + Send + Sync,
        Input: Into<String> + Send + Sync,
    {
        agent_ops::Extract::new(extractor)
    }

    /// Add a streaming prompt to the pipeline
    ///
    /// # Example
    /// ```rust
    /// use rig::{
    ///     pipeline::{self, StreamingOp},
    ///     providers::gemini::{self, completion::GEMINI_1_5_FLASH},
    ///     message::AssistantContent,
    /// };
    /// use futures::StreamExt;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = gemini::Client::from_env();
    /// let model = client.completion_model(GEMINI_1_5_FLASH);
    /// let streaming_model = pipeline::model_streaming(model);
    ///
    /// // Use pipeline::new() with streaming_prompt to create a streaming pipeline
    /// let pipeline = pipeline::new()
    ///     .streaming_prompt(streaming_model)
    ///     .map(|content| {
    ///         if let AssistantContent::Text(text) = content {
    ///             text.text
    ///         } else {
    ///             "".to_string()
    ///         }
    ///     });
    ///
    /// let mut stream = pipeline.stream("What is Rust?").await;
    /// while let Some(text) = stream.next().await {
    ///     print!("{}", text);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn streaming_prompt<P, R>(self, prompt: P) -> StreamingPipelineStage<P, R>
    where
        P: StreamingPromptTrait<R> + Send + Sync + Clone,
        R: Clone + Unpin,
    {
        StreamingPipelineStage::new(prompt)
    }

    /// Add a streaming prompt operation that sends chunks via a generic sender and returns the final response
    /// 
    /// This allows you to get real-time streaming updates while still working with the regular Op interface.
    /// Works with tokio::sync::mpsc::UnboundedSender or any custom sender that implements StreamingSender.
    ///
    /// # Example
    /// ```rust
    /// use rig::pipeline::{self, Op};
    /// use tokio::sync::mpsc;
    ///
    /// let (tx, mut rx) = mpsc::unbounded_channel();
    /// let agent = &openai_client.agent("gpt-4").build();
    ///
    /// let pipeline = pipeline::new()
    ///    .map(|name| format!("Tell me about: {name}"))
    ///    .streaming_prompt_with_sender(agent, tx);
    ///
    /// // Handle streaming chunks in a separate task
    /// tokio::spawn(async move {
    ///     while let Some(chunk) = rx.recv().await {
    ///         print!("{}", chunk);
    ///     }
    /// });
    ///
    /// // This will return the final complete response
    /// let result = pipeline.call("Rust programming".to_string()).await?;
    /// ```
    pub fn streaming_prompt_with_sender<P, Input, S>(
        self, 
        agent: P, 
        sender: S
    ) -> agent_ops::StreamingPromptWithSender<P, Input, S>
    where
        P: Send + Sync,
        Input: Send + Sync,
        S: agent_ops::StreamingSender,
        Self: Sized,
    {
        agent_ops::StreamingPromptWithSender::new(agent, sender)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("Failed to prompt agent: {0}")]
    PromptError(#[from] completion::PromptError),

    #[error("Failed to lookup documents: {0}")]
    LookupError(#[from] vector_store::VectorStoreError),
}

pub fn new() -> PipelineBuilder<ChainError> {
    PipelineBuilder {
        _error: std::marker::PhantomData,
    }
}

pub fn with_error<E>() -> PipelineBuilder<E> {
    PipelineBuilder {
        _error: std::marker::PhantomData,
    }
}

/// Creates a new streaming pipeline that passes through its input without modification.
/// Use with `.map()`, `.then()`, or `.chain()` to build streaming pipelines.
pub fn streaming<T>() -> impl StreamingOp<Input = T, Output = T>
where
    T: Send + Sync + Clone + 'static,
{
    StreamingPassthrough::new()
}

/// Create a new streaming pipeline that maps its input using the given function
pub fn streaming_map<F, Input, Output>(f: F) -> impl StreamingOp<Input = Input, Output = Output>
where
    F: Fn(Input) -> Output + Send + Sync + Clone + 'static,
    Input: Send + Sync + 'static,
    Output: Send + Sync + 'static,
{
    StreamingMap::new(f)
}

/// Create a new streaming pipeline that maps its input using the given async function
pub fn streaming_then<F, Input, Fut>(f: F) -> impl StreamingOp<Input = Input, Output = Fut::Output>
where
    F: Fn(Input) -> Fut + Send + Sync + Clone + 'static,
    Input: Send + Sync + 'static,
    Fut: Future + Send + Sync + 'static,
    Fut::Output: Send + Sync + 'static,
{
    StreamingThen::new(f)
}

/// Create a new streaming prompt
pub fn streaming_prompt<P, Input>(prompt: P) -> agent_ops::StreamingPrompt<P, Input>
where
    P: crate::streaming::StreamingPrompt<P::StreamingResponse> + crate::streaming::StreamingCompletionModel + Clone + Send + Sync + 'static,
    P::StreamingResponse: Clone + Unpin + Send + Sync + 'static,
    Input: Into<String> + Send + Sync + 'static,
{
    agent_ops::StreamingPrompt::new(prompt)
}

/// Create a new streaming prompt operation that sends chunks via a generic sender and returns the final response
pub fn streaming_prompt_with_sender<P, Input, S>(
    model: P, 
    sender: S
) -> agent_ops::StreamingPromptWithSender<P, Input, S>
where
    P: Send + Sync,
    Input: Send + Sync,
    S: agent_ops::StreamingSender,
{
    agent_ops::streaming_prompt_with_sender(model, sender)
}

/// A stage in a streaming pipeline that can transform streaming responses
pub struct StreamingPipelineStage<P, R> {
    prompt: P,
    _r: PhantomData<R>,
}

impl<P, R> StreamingPipelineStage<P, R>
where
    P: StreamingPromptTrait<R> + Send + Sync + Clone,
    R: Clone + Unpin,
{
    pub fn new(prompt: P) -> Self {
        Self {
            prompt,
            _r: PhantomData,
        }
    }

    /// Stream a prompt and return the streaming response
    pub async fn stream(
        &self, 
        input: impl Into<Message> + Send
    ) -> Result<StreamingCompletionResponse<R>, CompletionError> {
        self.prompt.stream_prompt(input).await
    }

    /// Apply a transformation to each item in the stream
    pub fn map<F, O>(self, f: F) -> StreamingMapStage<Self, F, R, O>
    where
        F: Fn(AssistantContent) -> O + Send + Sync + Clone + 'static,
        O: Send + 'static,
    {
        StreamingMapStage {
            source: self,
            mapper: f,
            _r: PhantomData,
            _o: PhantomData,
        }
    }
}

pub struct StreamingMapStage<S, F, R, O> {
    source: S,
    mapper: F,
    _r: PhantomData<R>,
    _o: PhantomData<O>,
}

impl<S, F, R, O> StreamingMapStage<S, F, R, O> {
    pub fn new(source: S, mapper: F) -> Self {
        Self {
            source,
            mapper,
            _r: PhantomData,
            _o: PhantomData,
        }
    }

    pub fn map<F2, O2>(self, f2: F2) -> StreamingMapStage<Self, F2, R, O2>
    where
        F2: Fn(O) -> O2 + Send + Sync + Clone + 'static,
        O2: Send + 'static,
        Self: Clone,
    {
        StreamingMapStage {
            source: self,
            mapper: f2,
            _r: PhantomData,
            _o: PhantomData,
        }
    }
}

// Implementation for nested StreamingMapStage with a StreamingPipelineStage inside
impl<P, F1, F2, R, O1, O2> StreamingMapStage<StreamingMapStage<StreamingPipelineStage<P, R>, F1, R, O1>, F2, R, O2>
where
    P: StreamingPromptTrait<R> + Send + Sync + Clone + 'static,
    F1: Fn(AssistantContent) -> O1 + Send + Sync + Clone + 'static,
    F2: Fn(O1) -> O2 + Send + Sync + Clone + 'static,
    R: Clone + Unpin + Send + Sync + 'static,
    O1: Send + 'static,
    O2: Send + 'static,
{
    pub async fn stream(
        &self, 
        input: impl Into<Message> + Send
    ) -> impl Stream<Item = O2> + Send {
        let converted_input = input.into();
        let source = self.source.clone();
        let inner_stream = source.stream(converted_input).await;
        let mapper = self.mapper.clone();
        
        Box::pin(inner_stream.map(move |item| (mapper)(item))) as Pin<Box<dyn Stream<Item = O2> + Send>>
    }
}

// Implementation specifically for when the source is StreamingPipelineStage
impl<P, F, R, O> StreamingMapStage<StreamingPipelineStage<P, R>, F, R, O>
where
    P: StreamingPromptTrait<R> + Send + Sync + Clone,
    F: Fn(AssistantContent) -> O + Send + Sync + Clone + 'static,
    R: Clone + Unpin + Send + Sync + 'static,
    O: Send + 'static,
{
    pub async fn stream(
        &self, 
        input: impl Into<Message> + Send
    ) -> impl Stream<Item = O> + Send {
        let source = self.source.clone();
        let result = source.stream(input).await;
        let mapper = self.mapper.clone();
        
        match result {
            Ok(response) => {
                Box::pin(response.map(move |result| {
                    match result {
                        Ok(content) => (mapper)(content),
                        Err(e) => panic!("Error in stream: {}", e), // Or handle more gracefully
                    }
                })) as Pin<Box<dyn Stream<Item = O> + Send>>
            },
            Err(e) => {
                eprintln!("Error starting stream: {}", e);
                Box::pin(futures::stream::empty()) as Pin<Box<dyn Stream<Item = O> + Send>>
            }
        }
    }
}

impl<P, R> Clone for StreamingPipelineStage<P, R>
where
    P: Clone,
{
    fn clone(&self) -> Self {
        Self {
            prompt: self.prompt.clone(),
            _r: PhantomData,
        }
    }
}

/// Create a new streaming pipeline with a streaming prompt
pub fn streaming_pipeline<P, R>(prompt: P) -> StreamingPipelineStage<P, R>
where
    P: StreamingPromptTrait<R> + Send + Sync + Clone,
    R: Clone + Unpin,
{
    StreamingPipelineStage::new(prompt)
}

/// Create a streaming adapter for a model
pub fn model_streaming<M>(model: M) -> StreamingModelAdapter<M>
where
    M: StreamingCompletionModel + Clone + Send + Sync + 'static,
{
    StreamingModelAdapter::new(model)
}

/// Adapter for using a StreamingCompletionModel directly with streaming pipelines
pub struct StreamingModelAdapter<M> {
    model: M,
}

impl<M> StreamingModelAdapter<M>
where
    M: StreamingCompletionModel + Clone + Send + Sync + 'static,
{
    pub fn new(model: M) -> Self {
        Self { model }
    }
}

impl<M> StreamingPromptTrait<M::StreamingResponse> for StreamingModelAdapter<M>
where
    M: StreamingCompletionModel + Clone + Send + Sync + 'static,
    M::StreamingResponse: Clone + Unpin,
{
    async fn stream_prompt(
        &self,
        prompt: impl Into<Message> + Send,
    ) -> Result<StreamingCompletionResponse<M::StreamingResponse>, CompletionError> {
        let msg = prompt.into();
        let request = completion::CompletionRequest { 
            chat_history: crate::OneOrMany::one(msg),
            preamble: None,
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            additional_params: None,
        };
        self.model.stream(request).await
    }
}

impl<M> Clone for StreamingModelAdapter<M>
where
    M: Clone,
{
    fn clone(&self) -> Self {
        Self {
            model: self.model.clone(),
        }
    }
}

impl<S, F, R, O> Clone for StreamingMapStage<S, F, R, O>
where
    S: Clone,
    F: Clone,
{
    fn clone(&self) -> Self {
        Self {
            source: self.source.clone(),
            mapper: self.mapper.clone(),
            _r: PhantomData,
            _o: PhantomData,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_ops::tests::{Foo, MockIndex, MockModel};
    use parallel::parallel;

    #[tokio::test]
    async fn test_prompt_pipeline() {
        let model = MockModel;

        let chain = super::new()
            .map(|input| format!("User query: {}", input))
            .prompt(model);

        let result = chain
            .call("What is a flurbo?")
            .await
            .expect("Failed to run chain");

        assert_eq!(result, "Mock response: User query: What is a flurbo?");
    }

    #[tokio::test]
    async fn test_prompt_pipeline_error() {
        let model = MockModel;

        let chain = super::with_error::<()>()
            .map(|input| format!("User query: {}", input))
            .prompt(model);

        let result = chain
            .try_call("What is a flurbo?")
            .await
            .expect("Failed to run chain");

        assert_eq!(result, "Mock response: User query: What is a flurbo?");
    }

    #[tokio::test]
    async fn test_lookup_pipeline() {
        let index = MockIndex;

        let chain = super::new()
            .lookup::<_, _, Foo>(index, 1)
            .map_ok(|docs| format!("Top documents:\n{}", docs[0].2.foo));

        let result = chain
            .try_call("What is a flurbo?")
            .await
            .expect("Failed to run chain");

        assert_eq!(result, "Top documents:\nbar");
    }

    #[tokio::test]
    async fn test_rag_pipeline() {
        let index = MockIndex;

        let chain = super::new()
            .chain(parallel!(
                passthrough(),
                agent_ops::lookup::<_, _, Foo>(index, 1),
            ))
            .map(|(query, maybe_docs)| match maybe_docs {
                Ok(docs) => format!("User query: {}\n\nTop documents:\n{}", query, docs[0].2.foo),
                Err(err) => format!("Error: {}", err),
            })
            .prompt(MockModel);

        let result = chain
            .call("What is a flurbo?")
            .await
            .expect("Failed to run chain");

        assert_eq!(
            result,
            "Mock response: User query: What is a flurbo?\n\nTop documents:\nbar"
        );
    }
}
