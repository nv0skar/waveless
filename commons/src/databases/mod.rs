// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

use object::*;

/// The maximum number of databases the user's application can connect to.
pub const DATABASE_LIMIT: usize = 9;

pub static DATABASES_CONNS: OnceLock<databases::DatabasesManager> = OnceLock::new();

pub type DbConns = HashMap<DatabaseId, Arc<dyn AnyDatabaseConnection>>;

/// The database's connections' pools manager.
#[derive(Constructor, Default, Debug)]
pub struct DatabasesManager {
    inner: DashMap<DatabaseId, Arc<dyn AnyDatabaseConnection>>,
    primary_id: RwLock<Option<CompactString>>,
}

impl DatabasesManager {
    pub fn acquire() -> &'static Self {
        DATABASES_CONNS.get_or_init(|| DatabasesManager::default())
    }
}

#[async_trait]
pub trait AnyDatabaseConnection: AnyExt {
    async fn execute(&self, input: DatabaseInput) -> Result<DatabaseOutput>;
}

#[async_trait]
pub trait DatabaseConsumer {
    fn databases(&self) -> CheapVec<DatabaseId>;

    async fn get_db_handle(&self) -> Result<DbConns> {
        let db_ids = self.databases();

        let _db_pool = DatabasesManager::acquire();

        Ok(match (db_ids.len(), _db_pool.primary_db().await) {
            (0, Ok(db_conn)) => HashMap::from([db_conn]),
            (0, Err(_)) => HashMap::from([]),
            _ => _db_pool.search_many(&db_ids)?,
        })
    }
}

#[derive(Debug)]
pub enum DatabaseInput {
    Query(CompactString),
    QueryValues(CompactString, CheapVec<CompactString, 8>),
    Bytes(Bytes),
    Any(Box<dyn Any + Send + Sync>),
}

#[derive(Debug)]
pub enum DatabaseOutput {
    Bytes(Bytes),
    Any(Box<dyn Any + Send + Sync>),
}

impl DatabasesManager {
    /// Loads the given database.
    #[instrument(skip_all)]
    pub async fn load(&self, db_config: project::DatabaseConfig) -> Result<()> {
        info!("Creating {}'s pool.", db_config.id());

        if self.inner.contains_key(db_config.id()) {
            Err(eyre!("The given database id already exists."))?
        }

        if self.primary_id.read().await.is_some() && *db_config.is_primary() {
            Err(eyre!("Cannot set two databases as primary.")
                .wrap_err("Delete first the primary database, then add this one."))?
        }

        let (pool, _) = db_config
            .connection()
            .new_conn(
                db_config.id().to_owned(),
                *db_config.pool_min_size(),
                *db_config.pool_max_size(),
            )
            .await?;

        if *db_config.is_primary() {
            *self.primary_id.write().await = Some(db_config.id().to_owned());
        }

        self.inner.insert(db_config.id().to_owned(), pool);

        Ok(())
    }

    /// Loads all the given databases into the pool.
    /// NOTE: if the pools hasn't been created yet it will be created in this call,
    /// also note that if it is already initialized the database will added to the pool.
    #[instrument(skip_all)]
    pub async fn load_all(&self, databases: CheapVec<project::DatabaseConfig>) -> Result<()> {
        if !self.inner.is_empty() {
            info!(
                "Dropping all previous database connections: {}",
                self.inner
                    .iter()
                    .map(|entry| entry.key().to_owned())
                    .join_compact(", ")
            );

            for key in self
                .inner
                .iter()
                .map(|entry| entry.key().to_owned())
                .collect::<CheapVec<_>>()
            {
                self.remove(&key).await?;
            }
        }

        // If more than one database is set, ensure there is exactly one marked as primary.
        if databases.len() > 1 {
            if databases
                .iter()
                .filter(|db| *db.is_primary())
                .collect::<CheapVec<_>>()
                .len()
                != 1
            {
                Err(eyre!("Cannot determine the primary database.").suggestion("When setting multiple database backends ensure there is exactly one set as primary."))?
            };
        }

        for db_config in databases {
            self.load(db_config).await?;
        }

        Ok(())
    }

    /// Remove a database given it's id.
    /// NOTE: if there is more that two databases and the target database
    /// is set as primary this operation will fail.
    #[instrument(skip_all)]
    pub async fn remove(&self, id: &DatabaseId) -> Result<()> {
        // Check whether the target database is primary.
        {
            let mut _primary_id_guard = self.primary_id.write().await;

            if let Some(primary_id) = _primary_id_guard.to_owned() {
                if primary_id == id {
                    if self.inner.len() > 2 {
                        Err(eyre!(
                            "Cannot remove primary database when there is more than 2 databases initialized."
                        ))?;
                    }

                    *_primary_id_guard = None;
                }
            }
        }

        let Some(_) = self.inner.remove(id) else {
            bail!("The given database id does not exist.")
        };

        Ok(())
    }

    /// Returns the primary database set for this project if any exists.
    /// NOTE: if there's only one database regardless of being marked as primary it will be returned.
    pub async fn primary_db(&self) -> Result<(DatabaseId, Arc<dyn AnyDatabaseConnection>)> {
        match (self.primary_id.read().await.to_owned(), self.inner.len()) {
            (Some(primary_id), _) => Ok((
                primary_id.to_owned(),
                self.inner
                    .get(&primary_id)
                    .wrap_err(format!(
                        "Unexpected error, the primary database {} hasn't been found in the pool.",
                        primary_id
                    ))?
                    .to_owned(),
            )),
            (None, 1) => Ok(self
                .inner
                .iter()
                .map(|entry| (entry.key().to_owned(), entry.value().to_owned()))
                .next()
                .unwrap()),
            _ => Err(eyre!(
                "No primary database has been defined for this project."
            )),
        }
    }

    /// Searches for the database given it's id.
    pub fn search(&self, id: &DatabaseId) -> Result<Arc<dyn AnyDatabaseConnection>> {
        self.inner
            .get(id)
            .ok_or(eyre!("Cannot find a database with the given id."))
            .map(|val| val.to_owned())
    }

    /// Searches for the databases given their ids.
    pub fn search_many(
        &self,
        ids: &CheapVec<DatabaseId>,
    ) -> Result<HashMap<CompactString, Arc<dyn AnyDatabaseConnection>>> {
        ids.as_ref()
            .iter()
            .map(|id| self.search(id).map(|db_conn| (id.to_owned(), db_conn)))
            .collect::<Result<HashMap<_, _>>>()
    }
}

impl Into<DbConns> for DatabasesManager {
    fn into(self) -> DbConns {
        (&self).into()
    }
}

impl<'a> Into<DbConns> for &'a DatabasesManager {
    fn into(self) -> DbConns {
        self.inner
            .iter()
            .map(|entry| (entry.key().to_owned(), entry.value().to_owned()))
            .collect::<HashMap<_, _>>()
    }
}

impl ObjectArtifact {
    pub async fn check_checksums_in_object(&self, db_conns: DbConns) -> Result<()> {
        for EndpointGeneratorChecksum { method, checksum } in self.databases_checksums() {
            // Run the endpoint generator.
            let (_, Some(current_checksum)) = method.generate(db_conns.to_owned()).await? else {
                return Err(eyre!("Endpoint generator did not return any checksum.")
                    .suggestion("Are you sure the generator is set to return a checksum?"));
            };

            if current_checksum != *checksum {
                bail!(
                    "The database schema has changed since the last build! Build the project again using the current schema."
                );
            } else {
                info!("Checksum `{}` verified.", hex::encode(checksum));
            }
        }
        Ok(())
    }
}
