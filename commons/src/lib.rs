// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

pub mod auth;
pub mod databases;
pub mod endpoint;
pub mod entry;
pub mod http_executor;
pub mod logging;
pub mod object;
pub mod project;
pub mod socket_executor;

#[cfg(not(target_arch = "wasm32"))]
pub mod runtime;

mod serialize_utils;

pub use serialize_utils::*;

use std::any::Any;
use std::cell::Cell;
use std::collections::HashMap;
use std::convert::Infallible;
use std::env::current_dir;
use std::fmt::Debug;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};

use boxed_any::*;
use boxed_any_derive::*;
use rustyrosetta::{codec::*, *};

use async_trait::*;
use bytes::Bytes as ConnBytes;
use color_eyre::Section;
use compact_str::*;
use dashmap::*;
use derive_builder::*;
use derive_more::{Constructor, Display};
use dyn_clone::*;
use eyre::{Context, ContextCompat, Result, bail, eyre};
use futures::future::BoxFuture;
use getset::*;
use http::StatusCode;
use http_body_util::combinators::BoxBody;
use hyper::Request;
use hyper_tungstenite::HyperWebsocket;
use iocraft::prelude::*;
use serde::{Deserialize, Serialize};
use serde_with::{IfIsHumanReadable, json::JsonString, serde_as};
use thiserror::*;
use tokio::sync::RwLock;
use tracing::*;

pub type ResultContext = CompactString; // TODO: Replace this with custom error types → `thiserror`

pub type DatabaseId = CompactString;
pub type DataSchemaDiscoveryMethodId = CompactString;
pub type ExternalDriverId = CompactString;

pub type ExecutionAtomId = CompactString;

pub type UserId = usize;

/// The binary's prefix.
pub const BINARY_MAGIC: &'static [u8] = b"waveless_binary";

thread_local! {
    pub static BINARY_MODE: Cell<bool> = const { Cell::new(false) }; // This will likely be fixed in the future. https://github.com/serde-rs/serde/issues/1732
}

pub trait AnyExt: Any + BoxedAny + DynClone + Send + Sync + Debug {
    fn name(&self) -> &str;
}

#[derive(Error, Debug)]
pub enum RequestError {
    #[error("Request error.")]
    Expected(StatusCode, CompactString),
    #[error(transparent)]
    Other(#[from] eyre::Error),
}

/// Tries to find the project's workspace root path.
pub fn get_workspace_root(project_file: &str) -> Result<PathBuf> {
    let mut current_dir = current_dir().unwrap();
    if current_dir.join(project_file).exists() {
        return Ok(current_dir);
    } else {
        while current_dir.pop() {
            if current_dir.join(project_file).exists() {
                return Ok(current_dir);
            }
        }
    };
    Err(eyre!(
        "The project's worspace root path cannot be determined."
    ))
}
