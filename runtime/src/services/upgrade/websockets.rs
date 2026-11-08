// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use waveless_commons::databases::DatabaseConsumer;

use crate::*;

/// TODO: add documentation.
#[derive(Clone, Constructor, Debug)]
pub struct WebSocketsSvc;

impl Service<RequestCx> for WebSocketsSvc {
    type Response = ResponseCx;

    type Error = RequestError;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[instrument(skip_all)]
    fn call(&mut self, mut cx: RequestCx) -> Self::Future {
        let future: Pin<_> = Box::pin(async move {
            let RequestCx {
                request,
                request_params,
                ..
            } = &mut cx;

            // Make sure the current request is truly a connection upgrade request.
            if !hyper_tungstenite::is_upgrade_request(&request) {
                unreachable!()
            }

            // Get the requested socket endpoint.
            let endpoint_id = match request
                .headers()
                .get("Sec-WebSocket-Protocol")
                .map(|value| value.to_str())
            {
                Some(Ok(value)) if !value.is_empty() => value,
                Some(Err(_)) => {
                    return Err(RequestError::Expected(
                        StatusCode::BAD_REQUEST,
                        format!("Socket endpoint id format is not valid.").into(),
                    ));
                }
                _ => {
                    return Err(RequestError::Expected(
                        StatusCode::BAD_REQUEST,
                        format!("Socket endpoint id was not specified.").into(),
                    ));
                }
            };

            let _build_lock = RuntimeCx::acquire().build().read().await;

            let Some(endpoint) = _build_lock
                .endpoints()
                .inner()
                .iter()
                .find(|endpoint| {
                    match (endpoint.id() == endpoint_id, endpoint.execution_target()) {
                        (true, ExecutionTarget::Socket(_)) => true,
                        _ => false,
                    }
                })
                .to_owned()
            else {
                return Err(RequestError::Expected(
                    StatusCode::BAD_REQUEST,
                    format!("Socket endpoint `{}` is not defined.", endpoint_id).into(),
                ));
            };

            let auth = endpoint.auth();

            // Enforce socket endpoint's authentication.
            // TODO: maybe do this in the [`SessionWatchdog`](waveless_runtime::services::SessionWatchdog) and enforce roles.
            if let AuthLevel::Required = auth.level()
                && request_params.get("user_id").is_none()
            {
                return Err(RequestError::Expected(
                    StatusCode::UNAUTHORIZED,
                    format!(
                        "Cannot open new socket connection for endpoint `{}`: invalid session.",
                        endpoint_id
                    )
                    .into(),
                ));
            }

            // Retrieves the endpoint's target database.
            let db_conns = endpoint.get_db_handle().await?;

            // Upgrade connection.
            debug!("Upgrading connection to WebSockets.");

            let (response, websocket) = hyper_tungstenite::upgrade(request, None)
                .expect("Cannot upgrade connection to WebSockets.");

            if let ExecutionTarget::Socket(socket_target) = endpoint.execution_target() {
                if let Some(execute_strategy) = socket_target.executor() {
                    let execute_strategy = execute_strategy.to_owned();

                    let request_params = request_params.to_owned();
                    let endpoint = endpoint.to_owned();

                    tokio::task::spawn(async move {
                        execute_strategy
                            .execute(
                                HandshakeCx {
                                    request_params,
                                    endpoint,
                                },
                                websocket,
                                db_conns,
                            )
                            .await
                    });
                } else {
                    return Err(RequestError::Expected(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!(
                            "The socket endpoint `{}` doesn't have any executor defined.",
                            endpoint.id()
                        )
                        .into(),
                    ));
                }
            } else {
                unreachable!()
            }

            return Ok(ResponseCx::new_with_status(
                StatusCode::SWITCHING_PROTOCOLS,
                response
                    .headers()
                    .iter()
                    .map(|(key, value)| (key.as_str().into(), value.to_str().unwrap().into()))
                    .collect::<HashMap<_, _>>(),
                None,
            ));
        })
        .into();

        future as Self::Future // Actually, this is not an error! https://github.com/rust-lang/rust/issues/92929
    }
}
