// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

use super::*;

/// Trait implemented for every session's storage backend.
/// TODO: add more docs.
#[typetag::serde]
#[async_trait]
pub trait AnySessionMethod: AnyExt + DatabaseConsumer {
    fn max_age(&self) -> Option<usize> {
        None
    }

    /// Get whether a user is authenticated by the given session token.
    /// If a user exists it will return it's associated id and optionally the authentication
    /// backend used to create the session.
    async fn check(
        &self,
        db_conns: DbConns,
        token: CompactString,
    ) -> Result<Option<(UserId, Option<AuthenticationBackendId>)>>;

    /// Create a new session for the given user.
    /// NOTE: you may optionally disable active user checks if you set `auth_backend` to `None`.
    async fn new(
        &self,
        db_conns: DbConns,
        user_id: UserId,
        auth_backend: Option<AuthenticationBackendId>,
    ) -> Result<CompactString>;

    /// Invalidate all session's of the given user.
    async fn invalidate(
        &self,
        db_conns: DbConns,
        user_id: UserId,
        token: Option<CompactString>,
    ) -> Result<()>;

    /// Remove all the expired sessions.
    async fn remove_expired(&self, db_conn: Arc<dyn AnyDatabaseConnection>) -> Result<()>;
}
