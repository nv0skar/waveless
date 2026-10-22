// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

pub mod conn_executor;
pub mod frontend_options;
pub mod internal_endpoints;
pub mod runtime_cx;
pub mod server;
pub mod services;

pub use conn_executor::*;
pub use internal_endpoints::*;
pub use runtime_cx::*;
pub use services::*;

use waveless_commons::*;

use waveless_commons::databases::*;
use waveless_commons::endpoint::*;
use waveless_commons::http_executor::{request_cx::*, *};
use waveless_commons::object::*;
use waveless_commons::socket_executor::handshake_cx::*;

use rustyrosetta::*;

use std::cell::LazyCell;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fs::read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

use bytes::Bytes as ConnBytes;
use clap::Subcommand;
use color_eyre::Section;
use compact_str::*;
use dashmap::DashMap;
use derive_more::Constructor;
use eyre::{Context, Result, eyre};
use futures::future::BoxFuture;
use getset::*;
use http::{HeaderName, HeaderValue, StatusCode};
use http_body_util::{BodyExt, Empty, Full, combinators::BoxBody};
use hyper::{Request, Response, body::Incoming};
use hyper_util::{
    rt::TokioIo, server::conn::auto::Builder as AutoHttpBuilder, service::TowerToHyperService,
};
use matchit::*;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject};
use serde_json::json;
use tokio::{
    fs::try_exists,
    sync::{OnceCell, RwLock},
};
use tokio_rustls::*;
use tower::{Layer, Service, ServiceBuilder, util::BoxCloneService};
use tower_governor::{governor::*, key_extractor::*};
use tower_http::{compression::*, cors::*, timeout::*};
use tracing::*;

pub type EndpointRouter = DashMap<HttpMethod, Router<Endpoint>>;

pub type ConnBody = BoxBody<ConnBytes, eyre::Error>;

pub static RUNTIME_CX: OnceCell<RuntimeCx> = OnceCell::const_new();

pub fn json_conn_body(value: &serde_json::Value) -> ConnBody {
    Full::new(ConnBytes::from(
        serde_json::to_vec_pretty(value).expect("Payload cannot be serialized into JSON."),
    ))
    .map_err(|_| unreachable!())
    .boxed()
}

pub fn empty_body() -> ConnBody {
    Empty::<ConnBytes>::new()
        .map_err(|_| unreachable!())
        .boxed()
}
