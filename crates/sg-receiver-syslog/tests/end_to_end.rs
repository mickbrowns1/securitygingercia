use sg_core::{Event, Receiver as _};
use sg_receiver_syslog::{SyslogConfig, SyslogReceiver};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

async fn collect_for(rx: &mut mpsc::Receiver<Event>, min: usize, timeout: Duration) -> Vec<Event> {
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

#[tokio::test(flavor = "multi_thread")]
async fn udp_receiver_parses_incoming_datagram() {
    let config = SyslogConfig::from_value(
        "syslog/udp",
        &serde_json::json!({
            "protocol": "udp",
            "listen_address": "127.0.0.1:19514",
        }),
    )
    .unwrap();

    let (tx, mut rx) = mpsc::channel(16);
    let shutdown = CancellationToken::new();
    let receiver = Box::new(SyslogReceiver::new("syslog/udp", config));
    let task_shutdown = shutdown.clone();
    let handle = tokio::spawn(async move { receiver.run(tx, task_shutdown).await });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket
        .send_to(
            b"<34>Oct 11 22:14:15 mymachine su: 'su root' failed for lonvick",
            "127.0.0.1:19514",
        )
        .await
        .unwrap();

    let events = collect_for(&mut rx, 1, Duration::from_secs(5)).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].attributes.get("hostname").unwrap(), "mymachine");
    assert_eq!(events[0].attributes.get("appname").unwrap(), "su");

    shutdown.cancel();
    handle.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_receiver_handles_octet_counting_across_multiple_messages() {
    let config = SyslogConfig::from_value(
        "syslog/tcp",
        &serde_json::json!({
            "protocol": "tcp",
            "listen_address": "127.0.0.1:19601",
            "framing": "octet_counting",
        }),
    )
    .unwrap();

    let (tx, mut rx) = mpsc::channel(16);
    let shutdown = CancellationToken::new();
    let receiver = Box::new(SyslogReceiver::new("syslog/tcp", config));
    let task_shutdown = shutdown.clone();
    let handle = tokio::spawn(async move { receiver.run(tx, task_shutdown).await });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let msg1 = "<34>1 2003-10-11T22:14:15.003Z mymachine su - ID47 - su root failed";
    let msg2 = "<34>1 2003-10-11T22:14:16.003Z mymachine su - ID48 - second message";
    let framed = format!("{} {}{} {}", msg1.len(), msg1, msg2.len(), msg2);

    use tokio::io::AsyncWriteExt;
    let mut stream = tokio::net::TcpStream::connect("127.0.0.1:19601")
        .await
        .unwrap();
    stream.write_all(framed.as_bytes()).await.unwrap();

    let events = collect_for(&mut rx, 2, Duration::from_secs(5)).await;
    assert_eq!(events.len(), 2, "expected both octet-counted frames to be parsed");
    assert_eq!(events[0].attributes.get("msgid").unwrap(), "ID47");
    assert_eq!(events[1].attributes.get("msgid").unwrap(), "ID48");

    shutdown.cancel();
    handle.await.unwrap().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn tcp_receiver_auto_detects_non_transparent_framing() {
    let config = SyslogConfig::from_value(
        "syslog/tcp",
        &serde_json::json!({
            "protocol": "tcp",
            "listen_address": "127.0.0.1:19602",
        }),
    )
    .unwrap();

    let (tx, mut rx) = mpsc::channel(16);
    let shutdown = CancellationToken::new();
    let receiver = Box::new(SyslogReceiver::new("syslog/tcp", config));
    let task_shutdown = shutdown.clone();
    let handle = tokio::spawn(async move { receiver.run(tx, task_shutdown).await });

    tokio::time::sleep(Duration::from_millis(100)).await;

    use tokio::io::AsyncWriteExt;
    let mut stream = tokio::net::TcpStream::connect("127.0.0.1:19602")
        .await
        .unwrap();
    stream
        .write_all(b"<34>Oct 11 22:14:15 mymachine su: first\n<34>Oct 11 22:14:16 mymachine su: second\n")
        .await
        .unwrap();

    let events = collect_for(&mut rx, 2, Duration::from_secs(5)).await;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].body.as_str().unwrap(), "first");
    assert_eq!(events[1].body.as_str().unwrap(), "second");

    shutdown.cancel();
    handle.await.unwrap().unwrap();
}
