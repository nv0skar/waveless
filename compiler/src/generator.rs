// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

//!
//! The Waveless' endpoints generator and schema's checksum calculator.
//! Connects to the specified the project's databases, scans their schema to produce
//! endpoints accordingly and produces the schema's checksum.
//!
use crate::*;

use databases::*;

#[derive(Clone, Constructor, Getters, Debug)]
#[getset(get = "pub")]
pub struct GeneratedEndpoints(CheapVec<(Bytes, (Endpoints, Option<EndpointGeneratorChecksum>))>);

impl GeneratedEndpoints {
    /// Discovers all endpoints from the project's database and calculate the checksum per database.
    /// TODO: Maybe the endpoint generation logic should be delegated to the [`AnyEndpointGenerator`](trait@waveless_commons::endpoint::generator::AnyEndpointGenerator) trait.
    #[instrument(skip_all)]
    pub async fn generate() -> Result<Self> {
        let cx = CompilerCx::acquire();

        let project = cx.project();

        let db_conns: DbConns = DatabasesManager::acquire().into();

        let mut all_endpoints = CheapVec::<_, 0>::new();

        let generators = project.compiler().endpoint_generators().iter();

        for generator in generators {
            let (endpoints, checksum) = generator.backend().generate(db_conns.to_owned()).await?;

            all_endpoints.push((
                generator.backend().id()?,
                (endpoints, {
                    if let Some(checksum) = checksum {
                        Some(EndpointGeneratorChecksum::new(
                            generator.backend().to_owned(),
                            checksum,
                        ))
                    } else {
                        None
                    }
                }),
            ));
        }

        Ok(GeneratedEndpoints::new(all_endpoints))
    }
}
