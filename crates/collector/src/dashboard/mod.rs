//! Live dashboard TUI: polls a running `sgcia run --status-addr`
//! process's status API once a second and renders per-receiver/pipeline/
//! exporter counters.

mod app;
mod client;
mod ui;

use app::{App, AppEvent};
use futures_util::StreamExt;
use std::net::SocketAddr;
use std::time::Duration;

pub async fn run(status_addr: SocketAddr) -> anyhow::Result<()> {
    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, status_addr).await;
    ratatui::restore();
    result
}

async fn run_app(
    terminal: &mut ratatui::DefaultTerminal,
    status_addr: SocketAddr,
) -> anyhow::Result<()> {
    let mut app = App::new(status_addr);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()?;
    let mut poll_interval = tokio::time::interval(Duration::from_secs(1));
    let mut crossterm_events = crossterm::event::EventStream::new();

    loop {
        terminal.draw(|f| ui::draw(f, &app))?;
        if app.should_quit {
            break;
        }

        tokio::select! {
            _ = poll_interval.tick() => {
                let result = client::fetch_status(&client, status_addr).await;
                app.update(AppEvent::PollResult(result));
            }
            maybe_event = crossterm_events.next() => {
                if let Some(Ok(crossterm::event::Event::Key(key))) = maybe_event {
                    app.update(AppEvent::Key(key));
                }
            }
        }
    }

    Ok(())
}
