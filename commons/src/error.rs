// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

/// Generic request error to use in `Any*` executor implementations.
/// `Expected` -> errors that are known to happen when handling a request.
///     - You might include additional info in `metadata`.
/// `Other` -> covers runtime errors, the backtrace is included in the response.
#[derive(Error, Debug)]
pub enum RequestError {
    #[error("Request error.")]
    Expected {
        status: StatusCode,
        message: CompactString,
        metadata: Option<MetadataKind>,
    },
    #[error(transparent)]
    Other(#[from] eyre::Error),
}

impl RequestError {
    pub fn new(status: StatusCode, message: CompactString) -> Self {
        Self::Expected {
            status,
            message,
            metadata: Default::default(),
        }
    }

    pub fn with_metadata(mut self, metadata: MetadataKind) -> Self {
        let Self::Expected {
            metadata: _metadata,
            ..
        } = &mut self
        else {
            return self;
        };

        *_metadata = Some(metadata);

        self
    }
}
