// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

pub type RouterServiceInner = BoxCloneService<Request<ConnBody>, Response<ConnBody>, Infallible>;

/// TODO: add documentation.
#[derive(Clone, Constructor)]
pub struct RouterService<S: Clone> {
    endpoints: S,
    frontend: Option<RouterServiceInner>,
}

impl<S> Service<Request<Incoming>> for RouterService<S>
where
    S: Service<RequestCx, Response = Response<ConnBody>, Error = Infallible>
        + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = Response<ConnBody>;

    type Error = Infallible;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        let frontend_ready = match &mut self.frontend {
            Some(frontend) => frontend.poll_ready(cx).is_ready(),
            None => true,
        };

        if self.endpoints.poll_ready(cx).is_ready() && frontend_ready {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }

    fn call(&mut self, request: Request<Incoming>) -> Self::Future {
        let request = request.map(|body| body.map_err(|err| eyre!(err)).boxed());

        let method = HttpMethod::from(request.method().as_str());

        let mut force_endpoint_id = Option::<CompactString>::None;

        // Check whether a connection upgrade to WebSockets is requested to steer the request to the connection handler.
        if hyper_tungstenite::is_upgrade_request(&request) {
            force_endpoint_id = Some(CONN_UPGRADE_WEBSOCKETS_ENDPOINT_ID.into());
        }

        let (endpoint, request_params) = {
            match force_endpoint_id {
                Some(target_endpoint_id) => {
                    let mut inner_endpoint_handler = self.to_owned().endpoints;

                    return Box::pin(async move {
                        let _build_lock = RuntimeCx::acquire().build().read().await;

                        let mut endpoints = _build_lock.endpoints().to_owned();

                        // Also add filtered internal endpoints.
                        endpoints.inner_mut().append(
                            &mut INTERNAL_ENDPOINTS
                                .iter()
                                .filter(|(kind, _)| {
                                    *kind == InternalEndpointKind::ConnectionUpgrade
                                })
                                .map(|(_, endpoint)| endpoint)
                                .cloned()
                                .collect::<CheapVec<_>>(),
                        );

                        // Search for endpoints that matches the given id.
                        let Some(endpoint) = endpoints
                            .inner()
                            .iter()
                            .find(|endpoint| endpoint.id() == target_endpoint_id)
                        else {
                            unreachable!()
                        };

                        inner_endpoint_handler
                            .call(RequestCx {
                                request,
                                method,
                                request_params: HashMap::new(),
                                endpoint: endpoint.to_owned(),
                            })
                            .await
                            .map_err(|_| unreachable!())
                    });
                }
                None => {
                    // Tries to match the route.
                    let route = request.uri().path().trim_matches('/').to_owned();

                    // Extracts the route from the method-aware router.
                    let Some(router) = RuntimeCx::acquire().router().get(&method) else {
                        if let Some(mut frontend_inner) = self.frontend.to_owned() {
                            return Box::pin(async move {
                                frontend_inner
                                    .call(request)
                                    .await
                                    .map_err(|_| unreachable!())
                            });
                        } else {
                            return Box::pin(async move {
                                let response = Response::builder()
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
                                                .http_cache_time())
                                                as u32
                                        ),
                                    );

                                Ok(response
                                    .status(404)
                                    .body(json_conn_body(&json!({
                                            "error": format!(
                                                "There is no route that accepts {}.",
                                                method
                                            )
                                        }
                                    )))
                                    .unwrap())
                            });
                        }
                    };

                    let Ok(matched) = router.at(&route) else {
                        if let Some(mut frontend_inner) = self.frontend.to_owned() {
                            debug!(
                                "Route `{}` is not defined, using the frontend service as a fallback.",
                                route
                            );

                            return Box::pin(async move {
                                frontend_inner
                                    .call(request)
                                    .await
                                    .map_err(|_| unreachable!())
                            });
                        } else {
                            return Box::pin(async move {
                                let response = Response::builder()
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
                                                .http_cache_time())
                                                as u32
                                        ),
                                    );

                                Ok(response
                                        .status(404)
                                        .body(
                                            json_conn_body(&json!({
                                                "error": format!(
                                                    "Route `{}` is not defined. HINT: Go to your project's endpoints folder and check the endpoint's routes.",
                                                    route)
                                            }))
                                    ).unwrap()
                                )
                            });
                        }
                    };

                    // Check whether the endpoint is reachable.
                    if !*matched.value.reachability() {
                        return Box::pin(async move {
                            let response = Response::builder()
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
                                            .http_cache_time())
                                            as u32
                                    ),
                                );

                            Ok(response
                                .status(302)
                                .body(json_conn_body(&json!({
                                    "error": format!(
                                        "Route `{}` is marked as unreachable.",
                                        route)
                                })))
                                .unwrap())
                        });
                    }

                    // Extracts the path's params.
                    let mut path_params = HashMap::<CompactString, ParamValue>::new();

                    for (key, value) in matched.params.iter() {
                        path_params.insert(key.into(), ParamValue::Client(Some(value.into())));
                    }

                    (matched.value.to_owned(), path_params)
                }
            }
        };

        let endpoint_fut = self.endpoints.call(RequestCx {
            request,
            method,
            request_params,
            endpoint,
        });

        return Box::pin(async move { endpoint_fut.await.map_err(|_| unreachable!()) });
    }
}
