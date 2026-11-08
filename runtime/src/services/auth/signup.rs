// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

/// TODO: add documentation.
#[derive(Clone, Constructor, Debug)]
pub struct SignUpSvc;

impl Service<RequestCx> for SignUpSvc {
    type Response = ResponseCx;

    type Error = RequestError;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[instrument(skip_all)]
    fn call(&mut self, cx: RequestCx) -> Self::Future {
        let future: Pin<_> = Box::pin(async move {
            let RequestCx { request, request_params, .. } = cx;

            let headers = request.headers();

            let request_params = request_params
                .iter()
                .filter_map(|entry| {
                    if let (key, ParamValue::Client(Some(value))) = entry {
                        Some((key.to_owned(), value.to_owned()))
                    } else {
                        None
                    }
                })
                .collect();

            let auth_config = RuntimeCx::acquire()
                .build()
                .read()
                .await
                .config()
                .authentication()
                .to_owned()
                .ok_or(RequestError::Other(eyre!(
                    "Authentication is not set for the current build."
                )))?;

            // Check whether signup is enabled for the current build.
            if !auth_config.allow_signup() {
                Err(RequestError::Other(eyre!("Signup is disabled for the current build.")))?;
            }

            let auth_method = {
                if auth_config.backends().len() == 1 {
                    auth_config.backends().first().unwrap()
                } else if let Some(auth_backend_name) = headers.get("AuthenticationType") {
                    // Checks for authentication type header if set.
                    if let Ok(auth_backend_name) = auth_backend_name.to_str() {
                        auth_config
                            .backends()
                            .iter()
                            .find(|auth_method| auth_method.name() == auth_backend_name)
                            .ok_or(RequestError::Expected(
                                StatusCode::BAD_REQUEST,
                                "Cannot find the requested authentication method."
                                    .into(),
                            ))?
                    } else {
                        return Err(RequestError::Expected(
                            StatusCode::BAD_REQUEST,
                            "Cannot deserialize the `AuthenticationMethod` header."
                                .into(),
                        ));
                    }
                } else {
                    return Err(RequestError::Expected(
                        StatusCode::BAD_REQUEST,
                        "No authentication method has been set. HINT: set one using the `AuthenticationMethod` header."
                            .into(),
                    ));
                }
            };

            let Ok(auth_db) = auth_method.get_db_handle().await else {
                return Err(RequestError::Other(eyre!(
                    "Cannot get the database connection for the authentication databases.",
                )));
            };

            // Create a new user.
            match auth_method.new(auth_db, request_params).await {
                Ok(user_id) => {
                    // Create a new session.
                    let session_method = auth_config.session();

                    let Ok(session_db) = session_method.get_db_handle().await else {
                        return Err(RequestError::Other(eyre!(
                            "Cannot get the database connection for the session databases.",
                        )));
                    };

                    let session_token =
                        session_method
                            .new(session_db, user_id)
                            .await
                            .map_err(|err| {
                                RequestError::Other(eyre!(
                                    "Cannot check the session token. {}",
                                    err
                                ))
                            })?;


                    let mut headers = HashMap::<CompactString, CompactString>::new();

                    // TODO: should add the secure param to `Set-Cookie`.
                    headers.insert(
                        "Set-Cookie".into(),
                        format!(
                            "Authorization={}; SameSite=Lax; Path=/; {}",
                            session_token,
                            session_method
                                .max_age()
                                .map(|max_age| format!("Max-Age={}", max_age))
                                .unwrap_or_default()
                        )
                        .into(),
                    );

                    Ok(ResponseCx::new(Default::default(), Some(BodyValue::Json(
                        json!({
                            "token": session_token
                        }),
                    ))))
                },

                Err(err) => Err(RequestError::Other(err))
            }
        }).into();

        future as Self::Future // Actually, this is not an error! https://github.com/rust-lang/rust/issues/92929
    }
}
