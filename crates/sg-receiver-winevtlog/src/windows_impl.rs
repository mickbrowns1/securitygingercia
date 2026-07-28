//! `EvtSubscribe`-based pull-model Windows Event Log receiver.
//!
//! UNVERIFIED: type-checked via `cargo check --target x86_64-pc-windows-msvc`
//! only -- there is no Windows host, live Event Log service, or wevtapi.dll
//! to actually run this against here. Treat every behavioral claim below as
//! "should work per the documented Win32 API contract," not "has been
//! observed to work." Run on a real Windows host or CI before trusting it.

use crate::bookmark;
use crate::config::{StartAt, WinEventLogConfig};
use async_trait::async_trait;
use sg_core::{Event, Receiver, SgError};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::System::EventLog::{
    EvtClose, EvtCreateBookmark, EvtNext, EvtRender, EvtSubscribe, EvtUpdateBookmark, EVT_HANDLE,
};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

// winevt.h flag values -- this version of windows-rs generates the
// EVT_SUBSCRIBE_FLAGS/EVT_RENDER_FLAGS types as bare newtype wrappers
// without named constants, so the documented numeric values are used
// directly here.
const EVT_SUBSCRIBE_TO_FUTURE_EVENTS: u32 = 1;
const EVT_SUBSCRIBE_START_AT_OLDEST_RECORD: u32 = 2;
const EVT_SUBSCRIBE_START_AFTER_BOOKMARK: u32 = 3;
const EVT_RENDER_EVENT_XML: u32 = 1;
const EVT_RENDER_BOOKMARK: u32 = 2;

pub struct WinEventLogReceiver {
    name: String,
    config: WinEventLogConfig,
}

impl WinEventLogReceiver {
    pub fn new(name: impl Into<String>, config: WinEventLogConfig) -> Self {
        Self {
            name: name.into(),
            config,
        }
    }
}

#[async_trait]
impl Receiver for WinEventLogReceiver {
    fn name(&self) -> &str {
        &self.name
    }

    async fn run(
        self: Box<Self>,
        tx: mpsc::Sender<Event>,
        shutdown: CancellationToken,
    ) -> Result<(), SgError> {
        let name = self.name.clone();
        let config = self.config.clone();
        let (raw_tx, mut raw_rx) = mpsc::channel::<String>(256);
        let blocking_shutdown = shutdown.clone();
        let blocking_name = name.clone();

        // EvtSubscribe's pull model (WaitForSingleObject + EvtNext) is
        // blocking Win32 API surface with no async equivalent, so it runs
        // on a blocking-pool thread and bridges into the async world via
        // a channel. `CancellationToken::is_cancelled()` is a plain sync
        // call, so the blocking loop can check it directly with no extra
        // bridging machinery.
        let blocking_handle = tokio::task::spawn_blocking(move || {
            run_blocking_subscription(blocking_name, config, raw_tx, blocking_shutdown);
        });

        loop {
            tokio::select! {
                biased;

                item = raw_rx.recv() => {
                    match item {
                        Some(xml) => {
                            let event = crate::xml::parse_event_xml(xml, &name);
                            if tx.send(event).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                _ = shutdown.cancelled() => break,
            }
        }

        let _ = blocking_handle.await;
        Ok(())
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn start_at_flag(start_at: StartAt) -> u32 {
    match start_at {
        StartAt::Beginning => EVT_SUBSCRIBE_START_AT_OLDEST_RECORD,
        StartAt::End => EVT_SUBSCRIBE_TO_FUTURE_EVENTS,
    }
}

unsafe fn create_bookmark(xml: Option<&str>) -> windows::core::Result<EVT_HANDLE> {
    match xml {
        Some(s) => {
            let wide = to_wide(s);
            EvtCreateBookmark(PCWSTR(wide.as_ptr()))
        }
        None => EvtCreateBookmark(PCWSTR(std::ptr::null())),
    }
}

/// Renders an event or bookmark handle to its XML representation via the
/// standard two-call pattern (query the required size, then fill it).
unsafe fn render(handle: EVT_HANDLE, flags: u32) -> windows::core::Result<String> {
    let mut buffer_used: u32 = 0;
    let mut property_count: u32 = 0;
    // Expected to report "insufficient buffer" here -- that's how this
    // API communicates the required size; only the second call's result
    // is treated as a real failure.
    let _ = EvtRender(
        EVT_HANDLE(0),
        handle,
        flags,
        0,
        None,
        &mut buffer_used,
        &mut property_count,
    );
    if buffer_used == 0 {
        return Ok(String::new());
    }

    let mut buffer: Vec<u16> = vec![0u16; buffer_used as usize / 2 + 1];
    let buffer_bytes = (buffer.len() * 2) as u32;
    EvtRender(
        EVT_HANDLE(0),
        handle,
        flags,
        buffer_bytes,
        Some(buffer.as_mut_ptr() as *mut core::ffi::c_void),
        &mut buffer_used,
        &mut property_count,
    )?;

    let len_u16 = (buffer_used as usize / 2).min(buffer.len());
    Ok(String::from_utf16_lossy(&buffer[..len_u16])
        .trim_end_matches('\0')
        .to_string())
}

fn run_blocking_subscription(
    name: String,
    config: WinEventLogConfig,
    raw_tx: mpsc::Sender<String>,
    shutdown: CancellationToken,
) {
    unsafe {
        let signal_event = match CreateEventW(None, true, false, PCWSTR(std::ptr::null())) {
            Ok(h) => h,
            Err(e) => {
                tracing::error!(receiver = %name, error = %e, "failed to create wait event");
                return;
            }
        };

        let existing_bookmark_xml = bookmark::load(&config.bookmark_file);
        let bookmark_result = match &existing_bookmark_xml {
            Some(xml) => create_bookmark(Some(xml)).map(|h| (h, EVT_SUBSCRIBE_START_AFTER_BOOKMARK)),
            None => create_bookmark(None).map(|h| (h, start_at_flag(config.start_at))),
        };
        let (bookmark_handle, subscribe_flags) = match bookmark_result {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!(receiver = %name, error = %e, "failed to recreate persisted bookmark, falling back to a fresh one");
                match create_bookmark(None) {
                    Ok(h) => (h, start_at_flag(config.start_at)),
                    Err(e) => {
                        tracing::error!(receiver = %name, error = %e, "failed to create a bookmark at all");
                        let _ = CloseHandle(signal_event);
                        return;
                    }
                }
            }
        };

        let channel_wide = to_wide(&config.channel);
        let query_wide = to_wide(&config.query);

        let subscription = match EvtSubscribe(
            EVT_HANDLE(0),
            signal_event,
            PCWSTR(channel_wide.as_ptr()),
            PCWSTR(query_wide.as_ptr()),
            bookmark_handle,
            None,
            None,
            subscribe_flags,
        ) {
            Ok(h) => h,
            Err(e) => {
                tracing::error!(receiver = %name, channel = %config.channel, error = %e, "EvtSubscribe failed");
                let _ = EvtClose(bookmark_handle);
                let _ = CloseHandle(signal_event);
                return;
            }
        };

        tracing::info!(receiver = %name, channel = %config.channel, "windows_eventlog subscription active (UNVERIFIED build)");

        let mut handles_buf = [0isize; 32];
        'outer: loop {
            if shutdown.is_cancelled() {
                break;
            }
            let wait = WaitForSingleObject(signal_event, 500);
            if wait == WAIT_TIMEOUT {
                continue;
            }
            if wait != WAIT_OBJECT_0 {
                tracing::warn!(receiver = %name, "WaitForSingleObject returned an unexpected result, retrying");
                continue;
            }
            let _ = ResetEvent(signal_event);

            loop {
                let mut returned: u32 = 0;
                match EvtNext(subscription, &mut handles_buf, 0, 0, &mut returned) {
                    Ok(()) => {
                        for &raw in &handles_buf[..returned as usize] {
                            let event_handle = EVT_HANDLE(raw);

                            match render(event_handle, EVT_RENDER_EVENT_XML) {
                                Ok(xml) => {
                                    if raw_tx.blocking_send(xml).is_err() {
                                        let _ = EvtClose(event_handle);
                                        let _ = EvtClose(subscription);
                                        let _ = EvtClose(bookmark_handle);
                                        let _ = CloseHandle(signal_event);
                                        return;
                                    }
                                }
                                Err(e) => {
                                    tracing::warn!(receiver = %name, error = %e, "EvtRender failed, skipping this event");
                                }
                            }

                            if let Err(e) = EvtUpdateBookmark(bookmark_handle, event_handle) {
                                tracing::warn!(receiver = %name, error = %e, "EvtUpdateBookmark failed");
                            } else if let Ok(bookmark_xml) = render(bookmark_handle, EVT_RENDER_BOOKMARK) {
                                if let Err(e) = bookmark::save(&config.bookmark_file, &bookmark_xml) {
                                    tracing::warn!(receiver = %name, error = %e, "failed to persist bookmark");
                                }
                            }

                            let _ = EvtClose(event_handle);
                        }
                        if shutdown.is_cancelled() {
                            break 'outer;
                        }
                    }
                    // Treated as "nothing more available right now"
                    // (the expected ERROR_NO_MORE_ITEMS case) rather than
                    // distinguished from a real error -- acceptable for
                    // this best-effort pull loop.
                    Err(_) => break,
                }
            }
        }

        let _ = EvtClose(subscription);
        let _ = EvtClose(bookmark_handle);
        let _ = CloseHandle(signal_event);
        tracing::info!(receiver = %name, "windows_eventlog subscription stopped");
    }
}
