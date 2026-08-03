mod dashboard;
mod editor;

use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;

/// sgcia -- companion dashboard/editor for an OTel-Collector-based
/// syslog/file/Windows-Event-Log pipeline. The collector engine itself is
/// `sgcia-otelcol` (built via `ocb --config otelcol/builder-config.yaml`);
/// this binary only edits its config and monitors it while it runs.
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
    /// Open the interactive config editor.
    Edit {
        /// Path to the YAML config file.
        #[arg(short, long)]
        config: PathBuf,
    },
    /// Open the live dashboard against a running `sgcia-otelcol` process
    /// (its statuscfg extension, specifically).
    Dashboard {
        /// Address of the status endpoint to poll.
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
        Command::Edit { config } => editor::run(config),
        Command::Dashboard { status_addr } => dashboard::run(status_addr).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_edit() {
        let cli = Cli::try_parse_from(["sgcia", "edit", "--config", "x.yaml"]).unwrap();
        assert!(matches!(cli.command, Command::Edit { .. }));
    }

    #[test]
    fn missing_config_is_a_parse_error_for_edit() {
        assert!(Cli::try_parse_from(["sgcia", "edit"]).is_err());
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
    fn dashboard_accepts_custom_status_addr() {
        let cli = Cli::try_parse_from(["sgcia", "dashboard", "--status-addr", "127.0.0.1:9999"]).unwrap();
        match cli.command {
            Command::Dashboard { status_addr } => {
                assert_eq!(status_addr, "127.0.0.1:9999".parse().unwrap())
            }
            other => panic!("expected Dashboard, got {other:?}"),
        }
    }
}
