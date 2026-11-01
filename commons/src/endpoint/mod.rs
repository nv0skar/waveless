// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

pub mod generator;

use crate::*;

use databases::*;

use http_executor::*;
use socket_executor::*;

/// Holds all the endpoints, is a wrapper of the [`CheapVec<Endpoint>`] type.
#[derive(Clone, PartialEq, Serialize, Deserialize, Getters, MutGetters, Debug)]
#[getset(get = "pub", get_mut = "pub")]
#[serde(default, transparent)]
pub struct Endpoints {
    #[serde(rename = "endpoints", skip_serializing_if = "should_skip_cheapvec")]
    inner: CheapVec<Endpoint, 0>,
}

impl Endpoints {
    /// Constructor that checks whether the given endpoints are valid.
    pub fn new(inner: CheapVec<Endpoint>) -> Result<Self> {
        for i in 0..inner.len() {
            for j in (i + 1)..inner.len() {
                if inner.get(i).unwrap() == inner.get(j).unwrap() {
                    bail!(
                        "An equivalent endpoint already exists: you were trying to add '{}', but '{}' is equivalent.",
                        inner.get(i).unwrap(),
                        inner.get(j).unwrap()
                    )
                }
            }
        }

        Ok(Self { inner })
    }

    /// NOTE: this constructor variant won't check whether the given endpoints are valid whatsoever.
    pub fn new_unchecked(inner: CheapVec<Endpoint>) -> Self {
        Self { inner }
    }

    /// Adds a new endpoint. This will check that there is no endpoint with the same method, route and version.
    pub fn add(&mut self, new_endpoint: Endpoint) -> Result<()> {
        let search = self.inner.iter().find(|endpoint| new_endpoint.eq(endpoint));

        match search {
            Some(endpoint) => Err(eyre!(
                "An equivalent endpoint already exists: you were trying to add '{}', but '{}' is equivalent.",
                new_endpoint,
                endpoint
            )),
            None => {
                self.inner.push(new_endpoint);
                Ok(())
            }
        }
    }

    /// Merges two endpoints buffers
    pub fn merge(&mut self, new_endpoints: Endpoints) -> Result<()> {
        for endpoint in new_endpoints.inner {
            if let Err(err) = self.add(endpoint.to_owned()) {
                bail!(
                    "Cannot add endpoint '{}' to the endpoints buffer. {}",
                    endpoint.id,
                    err.to_string()
                );
            }
        }
        Ok(())
    }
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            inner: Default::default(),
        }
    }
}

/// The main endpoint definition.
/// This will be then included in the Waveless project's binary.
#[derive(
    Clone, Serialize, Deserialize, Constructor, Builder, Getters, MutGetters, Display, Debug,
)]
#[display("{} ({:?}))", id, description)]
#[builder(default, pattern = "mutable", setter(strip_option))]
#[getset(get = "pub", get_mut = "pub")]
pub struct Endpoint {
    /// Endpoint's unique identifier
    id: CompactString,

    /// Sets the databases that this endpoint will operate on.
    #[serde(default, skip_serializing_if = "should_skip_cheapvec")]
    databases: CheapVec<DatabaseId>,

    /// Target variant.
    execution_target: ExecutionTarget,

    /// Sets the endpoint description.
    #[serde(default, skip_serializing_if = "should_skip_option")]
    description: Option<CompactString>,

    /// Sets the tags of this endpoint. By default the target table name will be adde as a tag.
    #[serde(default, skip_serializing_if = "should_skip_cheapvec")]
    tags: CheapVec<CompactString, 0>,

    /// Whether to require auth.
    auth: Auth,

    /// Whether this endpoint is deprecated.
    #[serde(default, skip_serializing_if = "should_skip")]
    deprecated: bool,
}

impl DatabaseConsumer for Endpoint {
    fn databases(&self) -> CheapVec<DatabaseId> {
        self.databases.to_owned()
    }
}

impl PartialEq for Endpoint {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id || self.execution_target == other.execution_target
    }
}

impl Default for Endpoint {
    fn default() -> Self {
        Self {
            id: "".into(),
            databases: Default::default(),
            execution_target: ExecutionTarget::Http(Default::default()),
            description: None,
            tags: CheapVec::new_const(),
            auth: Default::default(),
            deprecated: false,
        }
    }
}

#[derive(
    Clone, PartialEq, Serialize, Deserialize, Constructor, Builder, Getters, Display, Debug,
)]
#[display("{} (allowed roles: {:?}))", level, allowed_roles)]
#[builder(default, pattern = "mutable", setter(strip_option))]
#[getset(get = "pub")]
pub struct Auth {
    /// Whether to require auth.
    level: AuthLevel,

    /// All allowed roles to query the endpoint.
    #[serde(default, skip_serializing_if = "should_skip_cheapvec")]
    allowed_roles: CheapVec<CompactString, 0>,
}

impl Default for Auth {
    fn default() -> Self {
        Self {
            level: AuthLevel::None,
            allowed_roles: Default::default(),
        }
    }
}

/// Defines all levels of authentication an endpoint might require.
#[derive(Clone, PartialEq, Serialize, Deserialize, Display, Debug)]
pub enum AuthLevel {
    Required,
    InjectWhenAvailable,
    None,
}

/// Defines every kind of connection that a given endpoint may accept.
#[derive(Clone, PartialEq, Serialize, Deserialize, Display, Debug)]
pub enum ExecutionTarget {
    Http(HttpTarget),
    Socket(SocketTarget),
}

/// The HTTP endpoint definition that will be either created by the user or discovered by the compiler.
#[serde_as]
#[derive(
    Clone, Serialize, Deserialize, Constructor, Builder, Getters, MutGetters, Display, Debug,
)]
#[display("{} -> ({}, {:?})", route, method, version)]
#[builder(default, pattern = "mutable", setter(strip_option))]
#[getset(get = "pub", get_mut = "pub")]
pub struct HttpTarget {
    /// Route of the endpoint. Note that this will be prefixed with `{api_prefix}/{version}` (if version is set).
    route: CompactString,

    /// The version of the endpoint, if no version is set the endpoint will be accessible from `{api_prefix}/{route}`.
    #[serde(default, skip_serializing_if = "should_skip_option")]
    version: Option<CompactString>,

    /// Method of the endpoint
    method: HttpMethod,

    /// Establishes the execution pipeline. Note that if no executor is set, the server will try to handle the request internally.
    #[serde(default, skip_serializing_if = "should_skip_option")]
    execution_pipeline: Option<ExecutionAtom>,

    /// Sets the accepted query parameters.
    #[serde(default, skip_serializing_if = "should_skip_cheapvec")]
    query_params: CheapVec<CompactString, 0>,

    /// Sets the accepted body parameters.
    #[serde(default, skip_serializing_if = "should_skip_cheapvec")]
    body_params: CheapVec<CompactString, 0>,

    /// Whether to capture all the request's params.
    /// Useful for internal executors and generic trait implementations.
    #[serde(default, skip_serializing_if = "should_skip")]
    capture_all_params: bool,

    /// Whether this endpoint has been automatically generated.
    #[serde(default, skip_serializing_if = "auto_generated_skip")]
    auto_generated: bool,
}

/// Available HTTP methods
#[derive(Copy, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Display, Debug)]
#[serde(rename_all = "snake_case")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Unknown,
}

impl From<&str> for HttpMethod {
    fn from(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "get" => HttpMethod::Get,
            "post" => HttpMethod::Post,
            "put" => HttpMethod::Put,
            "delete" => HttpMethod::Delete,
            _ => HttpMethod::Unknown,
        }
    }
}

fn auto_generated_skip(value: &bool) -> bool {
    should_skip(&(!*value))
}

impl Into<ExecutionTarget> for HttpTarget {
    fn into(self) -> ExecutionTarget {
        ExecutionTarget::Http(self)
    }
}

impl PartialEq for HttpTarget {
    fn eq(&self, other: &Self) -> bool {
        self.route.trim().trim_matches('/') == other.route.trim().trim_matches('/')
            && self
                .version
                .to_owned()
                .map(|version| version.trim().trim_matches('/').to_owned())
                == other
                    .version
                    .to_owned()
                    .map(|version| version.trim().trim_matches('/').to_owned())
            && self.method == other.method
    }
}

impl Default for HttpTarget {
    fn default() -> Self {
        Self {
            route: "".into(),
            version: None,
            method: HttpMethod::Get,
            execution_pipeline: None.into(),
            query_params: Default::default(),
            body_params: Default::default(),
            capture_all_params: false,
            auto_generated: false,
        }
    }
}

/// The socket endpoint definition.
#[serde_as]
#[derive(
    Clone, Serialize, Deserialize, Constructor, Builder, Getters, MutGetters, Display, Debug,
)]
#[display("(Socket) {:?}", executor)]
#[builder(default, pattern = "mutable", setter(strip_option))]
#[getset(get = "pub", get_mut = "pub")]
pub struct SocketTarget {
    /// Establishes the endpoint handler.
    #[serde(default, skip_serializing_if = "should_skip_option")]
    #[serde_as(as = "IfIsHumanReadable<_, JsonString>")] // Explore müsli to avoid this.
    executor: Option<Arc<dyn AnySocketExecutor>>,
}

impl Into<ExecutionTarget> for SocketTarget {
    fn into(self) -> ExecutionTarget {
        ExecutionTarget::Socket(self)
    }
}

impl PartialEq for SocketTarget {
    fn eq(&self, _: &Self) -> bool {
        false
    }
}

impl Default for SocketTarget {
    fn default() -> Self {
        Self { executor: None }
    }
}
