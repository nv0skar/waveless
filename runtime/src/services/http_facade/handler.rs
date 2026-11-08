// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

/// TODO: add documentation.
#[derive(Clone, Debug)]
pub struct ExecuteHandler;

impl Service<RequestCx> for ExecuteHandler {
    type Response = ResponseCx;

    type Error = RequestError;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    /// Handles endpoints requests.
    fn call(&mut self, cx: RequestCx) -> Self::Future {
        let future: Pin<_> = Box::pin(async move {
            let RequestCx { endpoint, .. } = &cx;

            // Retrieves the endpoint's target database.
            let db_conns = endpoint.get_db_handle().await?;

            // Force the endpoint to have the HTTP target.
            let ExecutionTarget::Http(http_target) = endpoint.execution_target().to_owned() else {
                unreachable!()
            };

            let Some(execution_atom) = http_target.execution_pipeline() else {
                return Err(RequestError::Expected(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("The route doesn't have any executor defined. HINT: Go to your project's endpoints folder and check that '{}' has an executor set.", endpoint.id()).into(),
                ));
            };

            // Initialize the pipeline context.
            let mut pipeline_cx = PipelineCx::new(cx);

            // Initialize executor tracing context.
            let mut exec_tracing_cx = CheapVec::new();

            // Run the executors.
            let _start_time = Instant::now();

            pipeline_cx = execution_atom.execute(pipeline_cx, db_conns, &mut exec_tracing_cx).await?;

            let elapsed = _start_time.elapsed();

            debug!("Executed {} in {}ms ({} rounds).", exec_tracing_cx.iter().map(|id| format!("`{}`", id)).collect::<CheapVec<_>>().join(" → "), elapsed.as_millis(), exec_tracing_cx.len());

            let PipelineCx { response, .. } = pipeline_cx;

            Ok(response.unwrap_or_default())
        }).into();

        future as Self::Future // `rust-analyzer` complains here.
    }
}
