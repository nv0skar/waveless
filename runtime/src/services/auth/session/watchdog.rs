// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

/// TODO: add documentation.
#[derive(Clone, Constructor, Debug)]
pub struct SessionWatchdog<S>
where
    S: Service<RequestCx, Response = ResponseCx, Error = RequestError>,
{
    inner: S,
}

pub struct SessionWatchdogLayer;

impl<S> Layer<S> for SessionWatchdogLayer
where
    S: Service<RequestCx, Response = ResponseCx, Error = RequestError>,
{
    type Service = SessionWatchdog<S>;

    fn layer(&self, inner: S) -> Self::Service {
        SessionWatchdog { inner }
    }
}

impl<S> Service<RequestCx> for SessionWatchdog<S>
where
    S: Service<RequestCx, Response = ResponseCx, Error = RequestError> + Clone + Send + 'static,
    S::Future: Send + 'static,
    S::Response: Send + 'static,
    S::Error: Send + 'static,
{
    type Response = S::Response;

    type Error = S::Error;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[instrument(skip_all)]
    fn call(&mut self, mut cx: RequestCx) -> Self::Future {
        let mut inner = self.inner.to_owned();

        let future: Pin<_> = Box::pin(async move {
            let RequestCx {
                request,
                request_params,
                endpoint,
                ..
            } = &mut cx;

            let auth = endpoint.auth();

            let headers = request.headers();

            let _build_lock = RuntimeCx::acquire().build();

            let auth_config = _build_lock
                .read()
                .await
                .config()
                .authentication()
                .to_owned();

            if let Some(auth_config) = auth_config { 'auth_flow: {
                // TODO: the compiler should fail when including endpoints
                // that require authentication while not having
                // authentication set for the project.

                // Loads the session's method's and role's method's databases.
                let session_method = auth_config.session();

                let Ok(db_conns) = session_method.get_db_handle().await else {
                    return Err(RequestError::Other(eyre!(
                        "Cannot get the database connection for the session databases.",
                    )));
                };

                // Check the session.
                let token = match (headers.get("Authorization"), headers.get("Cookie")) {
                    (Some(auth_header), _) if !auth_header.is_empty() => {
                        if let Ok(token) = auth_header.to_str() {
                            Some(token)
                        } else {
                            // TODO: future connections from the same origin
                            // may be throttled.

                            return Err(RequestError::Expected(
                                StatusCode::BAD_REQUEST,
                                "Malformed auth header.".into(),
                            ));
                        }
                    }
                    (_, Some(cookie_header)) if !cookie_header.is_empty()=> {
                        if let Ok(cookies) = cookie_header.to_str() {
                            let cookies = cookies
                                .trim()
                                .split(';')
                                .map(|cookie| cookie.split_once('='));

                            let authorization_cookie = cookies
                                .filter(|opt| {
                                    opt.map(|(name, _)| name.to_lowercase() == "authorization")
                                        .unwrap_or(false)
                                })
                                .flatten()
                                .next();
                            if let Some((_, token)) = authorization_cookie {
                                Some(token)
                            } else {
                                None
                            }
                        } else {
                            // TODO: future connections from the same origin
                            // may be throttled.
                            return Err(RequestError::Expected(
                                StatusCode::BAD_REQUEST,
                                "Malformed cookie header.".into(),
                            ));
                        }
                    }
                    _ => None,
                };

                let Some(token) = token else {
                    if let AuthLevel::Required = auth.level() {
                        return Err(RequestError::Expected(
                            StatusCode::UNAUTHORIZED,
                            format!("'{}' requires authentication.", endpoint.id()).into(),
                        ));
                    }
                    else {
                        break 'auth_flow;
                    }
                };

                let session_check = session_method
                    .check(db_conns, token.into())
                    .await
                    .map_err(|err| {
                        RequestError::Other(eyre!("Cannot check the session token. {}", err))
                })?;

                match session_check {
                    Some(user_id) => {
                        // Inject user id if required.
                        match auth.level() {
                            AuthLevel::Required | AuthLevel::InjectWhenAvailable => {
                                request_params.insert(
                                    "user_id".into(),
                                    ParamValue::Internal(user_id.to_compact_string()),
                                );

                                request_params
                                    .insert("token".into(), ParamValue::Internal(token.into()));
                            },
                            _ => ()
                        }

                        if auth.allowed_roles().is_empty() {
                            break 'auth_flow;
                        } else {
                            let Some(role_method) = auth_config.role() else {
                                // TODO: the compiler should fail when including endpoints
                                // that requires roles while not having roles set for the project.
                                return Err(RequestError::Other(eyre!(
                                    "Endpoint '{}' requires roles authentication but they are not set for this build.",
                                    endpoint.id()
                                )));
                            };

                            let Ok(role_db) = role_method.get_db_handle().await else {
                                return Err(RequestError::Other(eyre!(
                                    "Cannot get the database connection for the roles databases."
                                )));
                            };

                            let Ok(role_check) = role_method.get(role_db, user_id).await else {
                                return Err(RequestError::Other(eyre!(
                                    "Cannot check the user's role."
                                )));
                            };

                            let Some(role) = role_check else {
                                return Err(RequestError::Expected(
                                    StatusCode::UNAUTHORIZED,
                                    "Current user does not have any role.".into(),
                                ));
                            };

                            if auth.allowed_roles().contains(&role.to_lowercase()) {
                                break 'auth_flow;
                            } else {
                                return Err(RequestError::Expected(
                                    StatusCode::UNAUTHORIZED,
                                    "Current user does not have any of the allowed roles.".into(),
                                ));
                            }
                        }
                    }
                    None if let AuthLevel::Required = auth.level() => {
                        return Err(RequestError::Expected(
                            StatusCode::UNAUTHORIZED,
                            "Invalid session.".into(),
                        ))
                    },
                    _ => (),
                }}
            } else {
                if let AuthLevel::Required = auth.level() {
                    Err(RequestError::Other(eyre!(
                        "Endpoint '{}' requires auth but authentication is not set for this build.",
                        endpoint.id()))
                    )?
                }
            }

            inner.call(cx).await
        })
        .into();

        future as Self::Future // `rust-analyzer` complains here.
    }
}
