use crossterm::event::{KeyCode, KeyEvent};
use sg_core::MetricsSnapshot;
use std::net::SocketAddr;
use std::time::Instant;

pub enum AppEvent {
    PollResult(Result<MetricsSnapshot, String>),
    Key(KeyEvent),
}

/// Pure dashboard state -- no terminal or network I/O here, so it's
/// entirely testable headlessly.
pub struct App {
    pub status_addr: SocketAddr,
    pub last_snapshot: Option<MetricsSnapshot>,
    pub last_error: Option<String>,
    pub last_poll_at: Option<Instant>,
    pub should_quit: bool,
}

impl App {
    pub fn new(status_addr: SocketAddr) -> Self {
        Self {
            status_addr,
            last_snapshot: None,
            last_error: None,
            last_poll_at: None,
            should_quit: false,
        }
    }

    pub fn update(&mut self, event: AppEvent) {
        match event {
            AppEvent::PollResult(Ok(snapshot)) => {
                self.last_snapshot = Some(snapshot);
                self.last_error = None;
                self.last_poll_at = Some(Instant::now());
            }
            // Keep showing the last good snapshot rather than blanking
            // the screen -- a transient connection failure shouldn't
            // erase the last thing we knew to be true.
            AppEvent::PollResult(Err(message)) => {
                self.last_error = Some(message);
                self.last_poll_at = Some(Instant::now());
            }
            AppEvent::Key(key) => self.handle_key(key),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) {
            self.should_quit = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sg_core::{ExporterSnapshot, PipelineSnapshot, ReceiverSnapshot};
    use std::collections::HashMap;

    fn addr() -> SocketAddr {
        "127.0.0.1:7801".parse().unwrap()
    }

    fn sample_snapshot() -> MetricsSnapshot {
        MetricsSnapshot {
            started_at: chrono::Utc::now(),
            uptime_seconds: 10,
            receivers: HashMap::from([(
                "syslog/udp".to_string(),
                ReceiverSnapshot { events_in: 5 },
            )]),
            pipelines: HashMap::from([(
                "logs/syslog".to_string(),
                PipelineSnapshot {
                    events_in: 5,
                    events_out: 4,
                    events_dropped: 1,
                    events_dead_lettered: 0,
                    parse_errors: 0,
                },
            )]),
            exporters: HashMap::from([(
                "sentinelone_hec".to_string(),
                ExporterSnapshot {
                    events_in: 4,
                    batches_sent: 1,
                    batches_failed: 0,
                    retries: 0,
                    last_error: None,
                },
            )]),
        }
    }

    #[test]
    fn successful_poll_clears_error_and_stores_snapshot() {
        let mut app = App::new(addr());
        app.update(AppEvent::PollResult(Err("boom".to_string())));
        assert!(app.last_error.is_some());

        app.update(AppEvent::PollResult(Ok(sample_snapshot())));
        assert!(app.last_error.is_none());
        assert!(app.last_snapshot.is_some());
    }

    #[test]
    fn failed_poll_keeps_last_good_snapshot() {
        let mut app = App::new(addr());
        app.update(AppEvent::PollResult(Ok(sample_snapshot())));
        app.update(AppEvent::PollResult(Err("connection refused".to_string())));

        assert_eq!(app.last_error.as_deref(), Some("connection refused"));
        assert!(app.last_snapshot.is_some(), "should keep the last good snapshot");
    }

    #[test]
    fn q_key_requests_quit() {
        let mut app = App::new(addr());
        app.update(AppEvent::Key(KeyEvent::from(KeyCode::Char('q'))));
        assert!(app.should_quit);
    }

    #[test]
    fn other_keys_do_not_quit() {
        let mut app = App::new(addr());
        app.update(AppEvent::Key(KeyEvent::from(KeyCode::Down)));
        assert!(!app.should_quit);
    }
}
