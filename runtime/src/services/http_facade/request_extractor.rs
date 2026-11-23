// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

/// TODO: add documentation.
#[derive(Clone, Constructor, Debug)]
pub struct RequestExtractor<S>
where
    S: Service<RequestCx, Response = ResponseCx, Error = RequestError>,
{
    inner: S,
}

pub struct RequestExtractorLayer;

impl<S> Layer<S> for RequestExtractorLayer
where
    S: Service<RequestCx, Response = ResponseCx, Error = RequestError>,
{
    type Service = RequestExtractor<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RequestExtractor { inner }
    }
}

impl<S> Service<RequestCx> for RequestExtractor<S>
where
    S: Service<RequestCx, Response = ResponseCx, Error = RequestError> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = S::Response;

    type Error = S::Error;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, mut cx: RequestCx) -> Self::Future {
        let mut inner = self.inner.to_owned();

        let future: Pin<_> = Box::pin(async move {
            let RequestCx {
                request,
                request_params,
                endpoint,
                ..
            } = &mut cx;

            let ExecutionTarget::Http(http_target) = endpoint.execution_target().to_owned() else {
                unreachable!()
            };

            // Searches for query params.
            if let Some(queries) = request.uri().query() {
                let queries = queries.split('&').map(|elem| {
                    elem.split_once('=')
                        .ok_or(eyre!("Cannot parse request's query."))
                        .unwrap()
                });
                if *http_target.capture_all_params() {
                    for (key, value) in queries {
                        request_params.insert(key.into(), ParamValue::Client(Some(value.into())));
                    }
                } else {
                    for key in http_target.query_params() {
                        let mut owned_iterator = queries.to_owned();
                        match owned_iterator.find(|elem| elem.0 == key) {
                            Some((key, value)) => request_params
                                .insert(key.into(), ParamValue::Client(Some(value.into()))),
                            None => request_params
                                .insert(key.to_compact_string(), ParamValue::Client(None)),
                        };
                    }
                }
            }

            // Searches for body params.
            if !http_target.body_params().is_empty() || *http_target.capture_all_params() {
                debug!("Request's body stream consumed. HINT: if you want to keep the TCP socket alive unset `capture_all_params` and empty `body_params`.");

                let body_bytes = Bytes::from_iter(
                    request
                        .collect()
                        .await
                        .map_err(|err| {
                            RequestError::new(
                                StatusCode::INTERNAL_SERVER_ERROR,
                                format!("Cannot get request's body. {}", err).into(),
                            )
                        })?
                        .to_bytes(),
                );

                if !body_bytes.is_empty() {
                    let Ok(json_body) = (match body_bytes.is_empty() {
                        true => Ok(serde_json::Value::Array(vec![])),
                        false => serde_json::from_slice::<serde_json::Value>(
                            body_bytes.iter().as_slice(),
                        ),
                    }) else {
                        return Err(RequestError::new(
                            StatusCode::BAD_REQUEST,
                            "Invalid request's body. The provided JSON's format is unsupported."
                                .into(),
                        ));
                    };

                    if *http_target.capture_all_params() {
                        for (key, value) in json_body.as_object().ok_or(RequestError::new(
                            StatusCode::BAD_REQUEST,
                            "Cannot extract the parameters from the request's body.".into(),
                        ))? {
                            request_params.insert(
                                key.into(),
                                ParamValue::Client(Some(
                                    value
                                        .as_str()
                                        .map(|s| s.to_string())
                                        .unwrap_or(value.to_string())
                                        .into(),
                                )),
                            );
                        }
                    } else {
                        for key in http_target.body_params() {
                            let value = {
                                match json_body
                                    .as_object()
                                    .ok_or(RequestError::new(
                                        StatusCode::BAD_REQUEST,
                                        "Cannot extract the parameters from the request's body."
                                            .into(),
                                    ))?
                                    .get(key.as_str())
                                {
                                    Some(res) => Some(
                                        res.as_str()
                                            .map(|s| s.to_string())
                                            .unwrap_or(res.to_string())
                                            .into(),
                                    ),
                                    None => None,
                                }
                            };

                            request_params.insert(key.to_owned(), ParamValue::Client(value));
                        }
                    }

                    // Reconstruct the request's body.
                    *request.body_mut() = BoxBody::new(
                        Full::new(ConnBytes::from_iter(body_bytes)).map_err(|_| unreachable!()),
                    );
                } else if !http_target.body_params().is_empty() {
                    return Err(RequestError::new(
                        StatusCode::BAD_REQUEST,
                        "The request's body for this endpoint cannot be empty.".into(),
                    ));
                }
            }

            inner.call(cx).await
        })
        .into();

        future as Self::Future // `rust-analyzer` complains here.
    }
}
