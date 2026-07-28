use sg_core::{Event, Receiver as _};
use sg_receiver_file::{FileLogConfig, FileLogReceiver};
use std::io::Write;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn collect_for(
    rx: &mut mpsc::Receiver<Event>,
    min: usize,
    timeout: Duration,
) -> Vec<Event> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    while out.len() < min && tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Some(ev)) => out.push(ev),
            Ok(None) => break,
            Err(_) => continue,
        }
    }
    out
}

fn spawn_receiver(
    name: &str,
    dir: &std::path::Path,
    checkpoint_path: &std::path::Path,
) -> (
    mpsc::Receiver<Event>,
    CancellationToken,
    tokio::task::JoinHandle<Result<(), sg_core::SgError>>,
) {
    let config = FileLogConfig::from_value(
        name,
        &serde_json::json!({
            "include": [format!("{}/*.log", dir.display())],
            "start_at": "beginning",
            "poll_interval": "50ms",
            "checkpoint_file": checkpoint_path.display().to_string(),
        }),
    )
    .unwrap();

    let (tx, rx) = mpsc::channel(16);
    let shutdown = CancellationToken::new();
    let receiver = Box::new(FileLogReceiver::new(name.to_string(), config));
    let task_shutdown = shutdown.clone();
    let handle = tokio::spawn(async move { receiver.run(tx, task_shutdown).await });
    (rx, shutdown, handle)
}

#[tokio::test(flavor = "multi_thread")]
async fn tails_file_applies_operator_chain_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("app.log");
    let checkpoint_path = dir.path().join("app.checkpoint.json");

    std::fs::write(
        &log_path,
        "%ASA-6-302013: first line\n%ASA-6-302014: second line\n",
    )
    .unwrap();

    let (mut rx, shutdown, handle) = spawn_receiver("filelog/app", dir.path(), &checkpoint_path);

    let events = collect_for(&mut rx, 2, Duration::from_secs(5)).await;
    assert_eq!(events.len(), 2, "expected both pre-existing lines to be tailed");

    // Prove the operator chain actually runs against tailed events.
    let mut defs = std::collections::HashMap::new();
    defs.insert(
        "extract".to_string(),
        serde_json::json!({
            "type": "regex",
            "pattern": r"^%ASA-(?P<severity>\d)-(?P<msgid>\d+):\s*(?P<message>.*)$",
            "parse_from": "body",
            "parse_to": "attributes",
        }),
    );
    let chain = sg_operators::build_chain(&defs, &["extract".to_string()]).unwrap();

    let mut processed = Vec::new();
    for ev in events {
        if let Some(p) = chain.run(ev).await {
            processed.push(p);
        }
    }
    assert_eq!(processed[0].attributes.get("msgid").unwrap(), "302013");
    assert_eq!(processed[1].attributes.get("msgid").unwrap(), "302014");

    shutdown.cancel();
    handle.await.unwrap().unwrap();

    // Append a third line, then start a brand-new receiver instance (fresh
    // channel, fresh in-memory state) pointed at the same checkpoint file.
    // Only the new line should come through -- proving the offset survived
    // the "restart".
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(f, "%ASA-6-302015: third line").unwrap();
    }

    let (mut rx2, shutdown2, handle2) =
        spawn_receiver("filelog/app", dir.path(), &checkpoint_path);
    let events2 = collect_for(&mut rx2, 1, Duration::from_secs(5)).await;

    assert_eq!(events2.len(), 1, "expected only the newly appended line");
    assert_eq!(
        String::from_utf8_lossy(&events2[0].raw),
        "%ASA-6-302015: third line"
    );

    shutdown2.cancel();
    handle2.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn rotation_starts_fresh_on_new_inode() {
    let dir = tempfile::tempdir().unwrap();
    let log_path = dir.path().join("app.log");
    let checkpoint_path = dir.path().join("app.checkpoint.json");
    std::fs::write(&log_path, "line-one\n").unwrap();

    let (mut rx, shutdown, handle) = spawn_receiver("filelog/app", dir.path(), &checkpoint_path);

    let events = collect_for(&mut rx, 1, Duration::from_secs(5)).await;
    assert_eq!(events.len(), 1);
    assert_eq!(String::from_utf8_lossy(&events[0].raw), "line-one");

    // Simulate logrotate: delete the file and recreate it under the same
    // name (new inode).
    std::fs::remove_file(&log_path).unwrap();
    std::fs::write(&log_path, "line-after-rotation\n").unwrap();

    let events2 = collect_for(&mut rx, 1, Duration::from_secs(5)).await;
    assert_eq!(events2.len(), 1);
    assert_eq!(
        String::from_utf8_lossy(&events2[0].raw),
        "line-after-rotation"
    );

    shutdown.cancel();
    handle.await.unwrap().unwrap();
}
