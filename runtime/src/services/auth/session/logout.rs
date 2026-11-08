// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

/// TODO: add documentation.
#[derive(Clone, Constructor, Debug)]
pub struct LogoutSvc;

impl Service<RequestCx> for LogoutSvc {
    type Response = ResponseCx;

    type Error = RequestError;

    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _cx: &mut std::task::Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    #[instrument(skip_all)]
    fn call(&mut self, cx: RequestCx) -> Self::Future {
        let future: Pin<_> = Box::pin(async move {
            let RequestCx { request_params, endpoint, .. } = cx;

            let ExecutionTarget::Http(http_target) = endpoint.execution_target() else {
                unreachable!()
            };

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

            let all_sessions = http_target.route().split("/").last().unwrap().to_lowercase() == "all";

            let user_id =
                match request_params
                    .get("user_id")
                    .ok_or(RequestError::Other(eyre!(
                        "Cannot logout as there is no session active.",
                    )))? {
                    ParamValue::Internal(user_id) => Ok(user_id.to_owned()),
                    _ => Err(RequestError::Expected(
                        StatusCode::FORBIDDEN,
                        "User id injection from the client is forbidden. HINT: if you are debugging your app you can try creating a new session manually.".into(),
                    )),
                }?.parse::<UserId>().map_err(|_| RequestError::Other(eyre!("Cannot convert user id to it's internal representation.")))?;


            let token =
                match request_params
                    .get("token")
                    .unwrap() {
                    ParamValue::Internal(token) if !all_sessions => Ok(Some(token.to_owned())),
                    ParamValue::Internal(_) if all_sessions => Ok(None),
                    _ => Err(RequestError::Expected(
                        StatusCode::FORBIDDEN,
                        "Session token injection from the client is forbidden. HINT: if you are debugging your app you can try creating a new session manually.".into(),
                    )),
                }?;

            let session_method = auth_config.session();

            let Ok(db_conns) = session_method.get_db_handle().await else {
                return Err(RequestError::Other(eyre!(
                    "Cannot get the database connection for the session databases.",
                )));
            };

            session_method.invalidate(db_conns, user_id, token).await?;

            Ok(ResponseCx::new(Default::default(), Some(BodyValue::Json(json!({})))))
        })
        .into();

        future as Self::Future // Actually, this is not an error! https://github.com/rust-lang/rust/issues/92929
    }
}
