use crate::build::{build_exporter, build_receiver};
use crate::metrics_relay::spawn_counting_relay;
use sg_core::{Event, Metrics, OperatorChain, PipelineMetrics};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Builds every exporter and pipeline from the resolved config, runs them
/// until a shutdown signal arrives, then cancels and waits for every task
/// to drain in-flight work before returning. If `status_addr` is set, also
/// serves the local status/monitoring API for that long (see
/// `crate::status_api`).
pub async fn run(cfg: sg_config::RawConfig, status_addr: Option<SocketAddr>) -> anyhow::Result<()> {
    let shutdown = CancellationToken::new();
    let mut tasks: Vec<tokio::task::JoinHandle<()>> = Vec::new();

    let metrics = Arc::new(Metrics::new(
        cfg.receivers.keys().cloned(),
        cfg.service.pipelines.keys().cloned(),
        cfg.exporters.keys().cloned(),
    ));

    // Exporters are built once, keyed by name -- multiple pipelines can
    // reference (and fan into) the same exporter instance.
    let mut exporter_senders: HashMap<String, mpsc::Sender<Arc<Event>>> = HashMap::new();
    for (name, value) in &cfg.exporters {
        let exporter = build_exporter(name, value, metrics.exporter(name))?;
        let (tx, rx) = mpsc::channel(1024);
        let task_shutdown = shutdown.clone();
        let exporter_name = name.clone();
        tasks.push(tokio::spawn(async move {
            if let Err(e) = exporter.run(rx, task_shutdown).await {
                tracing::error!(exporter = %exporter_name, error = %e, "exporter task ended with error");
            }
        }));
        exporter_senders.insert(name.clone(), tx);
    }

    for (pipeline_name, pipeline) in &cfg.service.pipelines {
        let chain = Arc::new(
            sg_operators::build_chain(&cfg.operators, &pipeline.operators)
                .map_err(|e| anyhow::anyhow!("pipeline '{pipeline_name}': {e}"))?,
        );

        let (recv_tx, recv_rx) = mpsc::channel::<Event>(4096);
        for receiver_name in &pipeline.receivers {
            let value = cfg.receivers.get(receiver_name).ok_or_else(|| {
                anyhow::anyhow!(
                    "pipeline '{pipeline_name}': receiver '{receiver_name}' not found"
                )
            })?;
            let receiver = build_receiver(receiver_name, value)?;
            // Each receiver gets its own inner channel; a counting relay
            // forwards into the pipeline's shared channel so per-receiver
            // event counts can be recorded without the Receiver trait
            // itself knowing anything about metrics.
            let (inner_tx, inner_rx) = mpsc::channel::<Event>(4096);
            let task_shutdown = shutdown.clone();
            let rname = receiver_name.clone();
            tasks.push(tokio::spawn(async move {
                if let Err(e) = receiver.run(inner_tx, task_shutdown).await {
                    tracing::error!(receiver = %rname, error = %e, "receiver task ended with error");
                }
            }));
            tasks.push(spawn_counting_relay(
                inner_rx,
                recv_tx.clone(),
                metrics.receiver(receiver_name),
            ));
        }
        // Drop the runner's own extra clone -- receivers' relays hold the
        // ones that matter. Once every relay ends, this channel closes
        // and the pipeline runner drains whatever is left, then exits.
        drop(recv_tx);

        let mut out_senders = Vec::new();
        for exporter_name in &pipeline.exporters {
            let sender = exporter_senders
                .get(exporter_name)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "pipeline '{pipeline_name}': exporter '{exporter_name}' not found"
                    )
                })?
                .clone();
            out_senders.push(sender);
        }

        let pipeline_shutdown = shutdown.clone();
        let pname = pipeline_name.clone();
        let pipeline_metrics = metrics.pipeline(pipeline_name);
        tasks.push(tokio::spawn(async move {
            run_pipeline(
                pname,
                recv_rx,
                chain,
                out_senders,
                pipeline_shutdown,
                pipeline_metrics,
            )
            .await;
        }));
    }

    if let Some(addr) = status_addr {
        let status_shutdown = shutdown.clone();
        let status_metrics = metrics.clone();
        let status_config = Arc::new(cfg.clone());
        tasks.push(tokio::spawn(async move {
            if let Err(e) =
                crate::status_api::serve(addr, status_metrics, status_config, status_shutdown)
                    .await
            {
                tracing::error!(%addr, error = %e, "status API server ended with error");
            }
        }));
        tracing::info!(%addr, "status API listening");
    }

    tracing::info!(
        pipelines = cfg.service.pipelines.len(),
        exporters = cfg.exporters.len(),
        "sgcia running -- press Ctrl-C to stop"
    );

    wait_for_shutdown_signal().await;
    tracing::info!("shutdown signal received, draining in-flight events");
    shutdown.cancel();

    for task in tasks {
        let _ = task.await;
    }
    tracing::info!("shutdown complete");
    Ok(())
}

async fn run_pipeline(
    name: String,
    mut rx: mpsc::Receiver<Event>,
    chain: Arc<OperatorChain>,
    out: Vec<mpsc::Sender<Arc<Event>>>,
    shutdown: CancellationToken,
    metrics: Arc<PipelineMetrics>,
) {
    loop {
        // Biased for the same reason as the exporter loops: drain an
        // already-buffered event before honoring shutdown.
        tokio::select! {
            biased;

            event = rx.recv() => {
                match event {
                    Some(event) => {
                        metrics.events_in.inc();
                        match chain.run(event).await {
                            Some(processed) => {
                                if !processed.meta.errors.is_empty() {
                                    metrics.parse_errors.add(processed.meta.errors.len() as u64);
                                }
                                if processed.meta.dead_letter {
                                    metrics.events_dead_lettered.inc();
                                }
                                metrics.events_out.inc();
                                let shared = Arc::new(processed);
                                for sender in &out {
                                    if sender.send(shared.clone()).await.is_err() {
                                        tracing::warn!(pipeline = %name, "an exporter channel closed early");
                                    }
                                }
                            }
                            None => {
                                metrics.events_dropped.inc();
                            }
                        }
                    }
                    None => break,
                }
            }
            _ = shutdown.cancelled() => break,
        }
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(error = %e, "failed to install SIGTERM handler, only Ctrl-C will stop sgcia");
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
