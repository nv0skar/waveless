// Waveless
// Copyright (C) 2026 Oscar Alvarez Gonzalez

use crate::*;

#[instrument(skip_all)]
pub async fn serve(
    addr: Option<SocketAddr>,
    tls_paths: Option<(PathBuf, PathBuf)>,
    frontend: Option<
        BoxCloneService<
            Request<BoxBody<ConnBytes, eyre::Error>>,
            Response<BoxBody<ConnBytes, eyre::Error>>,
            Infallible,
        >,
    >,
) -> Result<ResultContext> {
    let _build_lock = RuntimeCx::acquire().build();

    let listener = tokio::net::TcpListener::bind(
        addr.unwrap_or(
            _build_lock
                .read()
                .await
                .executor()
                .listening_addr()
                .ok_or(eyre!("No server address was provided."))?,
        ),
    )
    .await
    .unwrap();

    let tls_acceptor = match tls_paths {
        Some((cert_path, key_path))
            if try_exists(cert_path.to_owned()).await?
                && try_exists(key_path.to_owned()).await? =>
        {
            let cert = CertificateDer::pem_file_iter(cert_path)?.collect::<Result<Vec<_>, _>>()?;
            let key = PrivateKeyDer::from_pem_file(key_path)?;

            let mut tls_config = rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(cert, key)?;

            tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];

            Some(TlsAcceptor::from(Arc::new(tls_config)))
        }
        _ => None,
    };

    info!(
        "Executing '{}' on {} at {}",
        _build_lock.read().await.config().name(),
        listener.local_addr().unwrap(),
        chrono::Local::now()
    );

    let governor_conf = std::sync::Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(1000) // TODO: Make this a parameter in the 'project.toml'.
            .key_extractor(GlobalKeyExtractor) // TODO: Change this setting to allow IP-based rate limiting.
            .finish()
            .unwrap(),
    );

    // Cleans up the governor key pool.
    let governor_limiter = governor_conf.limiter().to_owned();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30)).await;
        debug!(
            "Cleaning up rate's limiter storage (size: {})",
            governor_limiter.len()
        );
        governor_limiter.retain_recent();
    });

    let governor = tower_governor::GovernorLayer::new(governor_conf).error_handler(|err| {
        Response::builder()
            .status(http::StatusCode::TOO_MANY_REQUESTS)
            .header("Content-Type", "application/json")
            .body(json_conn_body(&json!({
                "error": err.to_compact_string()
            })))
            .unwrap()
    });

    let compression = CompressionLayer::new().compress_when(predicate::SizeAbove::new(2048));

    let endpoint_svc = ServiceBuilder::new()
        .layer(ExecuteWrapperLayer)
        .layer(RequestExtractorLayer)
        .layer(SessionWatchdogLayer)
        .layer(UpgradeCaptureLayer)
        .layer(AuthCaptureLayer)
        .service(ExecuteHandler);

    let router = services::RouterService::new(endpoint_svc, frontend);

    let svc = ServiceBuilder::new()
        .layer(compression)
        .layer(CorsLayer::permissive())
        .layer(TimeoutLayer::with_status_code(
            http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(10),
        ))
        .layer(governor) // Rate limiting does not apply for cached requests.
        .service(router);

    let svc = TowerToHyperService::new(svc);

    loop {
        let (stream, _) = listener.accept().await?;

        let tls_acceptor = tls_acceptor.clone();

        let io = TokioIo::new(match tls_acceptor {
            Some(tls_acceptor) => match tls_acceptor.accept(stream).await {
                Ok(tls_stream) => tokio_util::either::Either::Left(tls_stream),
                Err(err) => {
                    error!("TLS handshake failed. {}", err);
                    continue;
                }
            },
            None => tokio_util::either::Either::Right(stream),
        });

        let svc = svc.to_owned();

        tokio::task::spawn(async move {
            let mut auto_http_builder = AutoHttpBuilder::new(ConnExecutor);

            auto_http_builder.http1().keep_alive(true);

            if let Err(err) = auto_http_builder
                .serve_connection_with_upgrades(io, svc)
                .await
            {
                error!(
                    "Internal error occurred while building a new connection handler: {:?}",
                    err
                );
            }
        });
    }
}
