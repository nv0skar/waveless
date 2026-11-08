// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

//!
//! The Waveless' frontend.
//!

use waveless_commons::{logging::*, object::*, runtime::handle_main, *};
use waveless_compiler::{compiler_cx::*, new::*, workspace::*};
use waveless_runtime::{frontend_options::*, server::serve, *};

use databases::*;

use rustyrosetta::*;

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{ColorChoice, Parser, Subcommand, builder::styling::*};
use compact_str::*;
use eyre::{Result, eyre};
use mimalloc::MiMalloc;
use nestify::nest;
use tracing::*;

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn command_styles() -> Styles {
    Styles::styled()
        .header(AnsiColor::Green.on_default() | Effects::BOLD)
        .usage(AnsiColor::Green.on_default() | Effects::BOLD)
        .literal(AnsiColor::Cyan.on_default() | Effects::BOLD)
        .placeholder(AnsiColor::Yellow.on_default())
}

nest! {
    #[derive(Parser)]
    #[command(
        name = "waveless",
        version,
        about = "The Waveless' frontend: build & execute.",
        long_about = "Analyze and build the project in the current directory and generate a Waveless' binary.",
        propagate_version = true,
        subcommand_required = true,
        arg_required_else_help = true,
        color = ColorChoice::Auto,
        styles = command_styles()
    )]
    struct Frontend {
        /// Whether to enable debug mode in the compiler.
        #[arg(short = 'D', long = "debug", default_value_t = false)]
        debug: bool,

        /// All cli subcommands
        #[command(subcommand)]
        subcommand: Option<
            #[derive(Subcommand)]
            enum Subcommands {
                /// Creates a new Waveless' project.
                #[command(about = "Creates a new Waveless' project.")]
                New {
                    #[arg(help = "Project's name")]
                    name: CompactString,
                },

                /// Builds and launches the server executor using the outputted binary.
                #[command(about = "Builds and launches the server executor using the outputted binary.")]
                Run {
                    #[arg(help = "Listening address.")]
                    addr: Option<SocketAddr>,

                    #[arg(long = "tls_cert", help = "TLS cert path.")]
                    tls_cert: Option<PathBuf>,

                    #[arg(long = "tls_cert_key", help = "TLS cert key path.")]
                    tls_cert_key: Option<PathBuf>,
                },

                /// Builds the current project.
                #[command(about = "Builds the current project.")]
                Build,

                /// The Waveless' executor.
                #[command(about = "The Waveless' executor.", subcommand)]
                Executor(ExecutorFrontendOptions)
            }
        >
    }
}

fn main() -> Result<()> {
    handle_main(try_main)
}

async fn try_main() -> Result<ResultContext> {
    waveless_sql::register(); // Do not strip `waveless_sql` at build.

    let cli = Frontend::parse();

    // Setup logging
    subscribe_logging(cli.debug, CheapVec::new())?;

    // Handle frontend subcommands
    match cli.subcommand {
        Some(Subcommands::New { name }) => new_project(name),
        Some(Subcommands::Run {
            addr,
            tls_cert,
            tls_cert_key,
        }) => {
            CompilerCx::set_cx(CompilerCx::from_workspace().await?);

            let _config_lock = CompilerCx::acquire().project().config();

            DatabasesManager::acquire()
                .load_all(_config_lock.databases().to_owned())
                .await?;

            let build = load::<ObjectArtifact>().await?.left().unwrap();

            RuntimeCx::set_cx(RuntimeCx::from_build(build).await?);

            let _build_lock = RuntimeCx::acquire().build();

            if *_build_lock
                .read()
                .await
                .executor()
                .check_databases_cheksums()
            {
                warn!("Skipping databases' schema checksum verification.");
            }

            let tls_paths = match (tls_cert, tls_cert_key) {
                (Some(tls_cert), Some(tls_cert_key)) => Some((tls_cert, tls_cert_key)),
                _ => None,
            };

            serve(addr, tls_paths, None).await?;

            return Ok("".into());
        }
        Some(Subcommands::Build) => {
            CompilerCx::set_cx(CompilerCx::from_workspace().await?);

            let _config_lock = CompilerCx::acquire().project().config();

            DatabasesManager::acquire()
                .load_all(_config_lock.databases().to_owned())
                .await?;

            let buff = load::<Bytes>().await?.right().unwrap();

            binary_file_from_buff(buff)
        }
        Some(Subcommands::Executor(executor_options)) => match executor_options {
            ExecutorFrontendOptions::Run {
                path,
                addr,
                tls_cert,
                tls_cert_key,
            } => {
                RuntimeCx::set_cx(RuntimeCx::from_path(path).await?);

                let _build_lock = RuntimeCx::acquire().build();

                DatabasesManager::acquire()
                    .load_all(_build_lock.read().await.config().databases().to_owned())
                    .await?;

                if *_build_lock
                    .read()
                    .await
                    .executor()
                    .check_databases_cheksums()
                {
                    _build_lock
                        .read()
                        .await
                        .check_checksums_in_object(DatabasesManager::acquire().to_owned().into())
                        .await?;
                }

                let tls_paths = match (tls_cert, tls_cert_key) {
                    (Some(tls_cert), Some(tls_cert_key)) => Some((tls_cert, tls_cert_key)),
                    _ => None,
                };

                serve(addr, tls_paths, None).await?;

                Ok("".into())
            }
        },
        None => Err(eyre!("No subcommdand provided!")),
    }
}
