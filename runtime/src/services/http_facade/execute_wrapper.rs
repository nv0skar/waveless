// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

/// TODO: add documentation.
#[derive(Clone, Constructor, Debug)]
pub struct ExecuteWrapper<S>
where
    S: Service<RequestCx, Error = RequestError>,
{
    inner: S,
}

pub struct ExecuteWrapperLayer;

impl<S> Layer<S> for ExecuteWrapperLayer
where
    S: Service<RequestCx, Response = ResponseCx, Error = RequestError>,
{
    type Service = ExecuteWrapper<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ExecuteWrapper { inner }
    }
}

impl<S> Service<RequestCx> for ExecuteWrapper<S>
where
    S: Service<RequestCx, Response = ResponseCx, Error = RequestError> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response<ConnBody>;

    type Error = Infallible;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(|_| unreachable!())
    }

    /// Handles endpoints requests.
    #[instrument(skip_all)]
    fn call(&mut self, cx: RequestCx) -> Self::Future {
        let RequestCx { request, .. } = &cx;

        info!(
            "`{}` request at `{}` {}",
            request.method(),
            request.uri().path(),
            request
                .headers()
                .get("host")
                .map(|val| format!("from `{}`", val.to_str().unwrap_or_default()))
                .unwrap_or_default()
        );

        let fut = self.inner.call(cx);

        Box::pin(async move {
            let mut response = Response::builder()
                .header("Content-Type", "application/json; charset=utf-8")
                .header(
                    "Cache-Control",
                    format!(
                        "max-age={}",
                        (*RuntimeCx::acquire()
                            .build()
                            .read()
                            .await
                            .executor()
                            .http_cache_time()) as u32
                    ),
                );

            match fut.await {
                Ok(execute_response) => {
                    let headers = response.headers_mut().unwrap();

                    for (key, value) in execute_response.headers() {
                        headers.insert(
                            HeaderName::from_bytes(key.as_bytes()).unwrap(),
                            HeaderValue::from_bytes(value.as_bytes()).unwrap(),
                        );
                    }

                    match execute_response.body() {
                        Some(BodyValue::Json(value)) => Ok(response
                            .status(execute_response.status())
                            .body(json_conn_body(value))
                            .unwrap()),
                        Some(BodyValue::Any(encode)) => Ok(response
                            .status(execute_response.status())
                            .body(json_conn_body(&json!({
                                "data": encode.encode().unwrap()
                            })))
                            .unwrap()),
                        _ => Ok(response
                            .status(execute_response.status())
                            .body(empty_body())
                            .unwrap()),
                    }
                }
                Err(err) => Ok(response
                    .status({
                        match err {
                            RequestError::Expected(status, _) => status,
                            RequestError::Other(_) => StatusCode::INTERNAL_SERVER_ERROR,
                        }
                    })
                    .body(
                        json_conn_body(&json!({
                            "error": match &err {
                                RequestError::Expected(_, err) => err.to_owned(),
                                RequestError::Other(err) => err.to_owned().to_compact_string(),
                            },
                            "trace": match &err {
                                RequestError::Other(err) => err.chain().skip(1).map(|err| err.to_string()).collect::<CheapVec<_>>(),
                                _ => CheapVec::from_iter([])
                            }
                        }))
                        .map_err(|err| eyre!(err))
                        .boxed(),
                    )
                    .unwrap()),
            }
        })
    }
}
