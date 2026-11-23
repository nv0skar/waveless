// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

mod role;
mod session;

pub use role::*;
pub use session::*;

use crate::*;

use databases::*;

/// Trait implemented for every user authentication mechanism.
/// Note that the auth data does not have to live in a SQL database...
/// TODO: add more docs.
#[typetag::serde]
#[async_trait]
pub trait AnyAuthenticationMethod: AnyExt + DatabaseConsumer {
    /// Check whether the given credentials match for a given user.
    async fn check(
        &self,
        db_conns: DbConns,
        entries: HashMap<CompactString, CompactString>,
    ) -> Result<Option<UserId>>;

    /// Signup a new user.
    async fn new(
        &self,
        db_conns: DbConns,
        entries: HashMap<CompactString, CompactString>,
    ) -> Result<UserId>;

    /// Deletes a user given it's id.
    async fn delete(&self, db_conns: DbConns, user_id: UserId) -> Result<()>;

    /// Check whether a user is active.
    async fn active(&self, db_conns: DbConns, user_id: UserId) -> Result<bool>;
}
