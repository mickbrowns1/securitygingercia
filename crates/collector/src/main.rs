mod build;
mod dashboard;
mod editor;
mod metrics_relay;
mod pipeline;
mod status_api;

use clap::{Parser, Subcommand};
use sg_config::{component_type, RawConfig};
use std::net::SocketAddr;
use std::path::PathBuf;

/// Security Ginger Collect It All (sgcia) -- syslog, flat-file, and Windows
/// Event Log collector with a built-in parsing pipeline.
#[derive(Parser, Debug)]
#[command(name = "sgcia", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Tracing filter, e.g. "info", "sgcia=debug".
    #[arg(long, default_value = "info", global = true)]
    log_level: String,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run the collector.
    Run {
        /// Path to the YAML config file.
        #[arg(short, long)]
        config: PathBuf,
        /// Bind address for the local status HTTP API (omit to disable).
        #[arg(long)]
        status_addr: Option<SocketAddr>,
    },
    /// Validate the config and print the resolved pipeline graph, then exit.
    Check {
        /// Path to the YAML config file.
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Open the interactive config editor.
    Edit {
        /// Path to the YAML config file.
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Open the live dashboard against a running `sgcia run` process.
    Dashboard {
        /// Address of the status API to poll.
        #[arg(long, default_value = "127.0.0.1:7801")]
        status_addr: SocketAddr,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(&cli.log_level))
        .init();

    match cli.command {
        Command::Run { config, status_addr } => {
            let cfg = sg_config::load_file(&config)?;
            print_pipeline_graph(&cfg);
            pipeline::run(cfg, status_addr).await
        }
        Command::Check { config } => {
            let cfg = sg_config::load_file(&config)?;
            print_pipeline_graph(&cfg);
            println!("\nconfig OK");
            Ok(())
        }
        Command::Edit { config } => editor::run(config),
        Command::Dashboard { status_addr } => dashboard::run(status_addr).await,
    }
}

fn print_pipeline_graph(cfg: &RawConfig) {
    println!("Resolved pipeline graph:");
    let mut names: Vec<&String> = cfg.service.pipelines.keys().collect();
    names.sort();
    for name in names {
        let pipeline = &cfg.service.pipelines[name];
        println!("  pipeline: {name}");
        println!("    receivers: {}", describe(&pipeline.receivers));
        println!("    operators: {}", describe(&pipeline.operators));
        println!("    exporters: {}", describe(&pipeline.exporters));
    }
}

fn describe(ids: &[String]) -> String {
    if ids.is_empty() {
        return "(none)".to_string();
    }
    ids.iter()
        .map(|id| format!("{id} [{}]", component_type(id)))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_run_with_status_addr() {
        let cli = Cli::try_parse_from([
            "sgcia",
            "run",
            "--config",
            "x.yaml",
            "--status-addr",
            "127.0.0.1:7801",
        ])
        .unwrap();
        match cli.command {
            Command::Run { config, status_addr } => {
                assert_eq!(config, PathBuf::from("x.yaml"));
                assert_eq!(status_addr, Some("127.0.0.1:7801".parse().unwrap()));
            }
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parses_run_without_status_addr() {
        let cli = Cli::try_parse_from(["sgcia", "run", "--config", "x.yaml"]).unwrap();
        match cli.command {
            Command::Run { status_addr, .. } => assert_eq!(status_addr, None),
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn parses_check() {
        let cli = Cli::try_parse_from(["sgcia", "check", "--config", "x.yaml"]).unwrap();
        assert!(matches!(cli.command, Command::Check { .. }));
    }

    #[test]
    fn parses_edit() {
        let cli = Cli::try_parse_from(["sgcia", "edit", "--config", "x.yaml"]).unwrap();
        assert!(matches!(cli.command, Command::Edit { .. }));
    }

    #[test]
    fn dashboard_defaults_status_addr() {
        let cli = Cli::try_parse_from(["sgcia", "dashboard"]).unwrap();
        match cli.command {
            Command::Dashboard { status_addr } => {
                assert_eq!(status_addr, "127.0.0.1:7801".parse().unwrap())
            }
            other => panic!("expected Dashboard, got {other:?}"),
        }
    }

    #[test]
    fn missing_config_is_a_parse_error() {
        assert!(Cli::try_parse_from(["sgcia", "run"]).is_err());
    }
}
