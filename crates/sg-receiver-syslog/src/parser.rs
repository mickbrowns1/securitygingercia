use crate::config::RfcMode;
use chrono::{Datelike, Local, Utc};
use serde_json::{json, Map, Value};
use sg_core::{Event, Severity};
use syslog_loose::{Message, Protocol as SyslogProtocol};

/// Parses a raw syslog message (already de-framed) into an `Event`.
/// Falls back gracefully on parse failure: the raw text becomes the body
/// as-is and the failure is recorded in `event.meta.errors`, rather than
/// dropping the datagram/frame outright.
pub fn parse_into_event(raw: bytes::Bytes, rfc: RfcMode, receiver_name: &str) -> Event {
    let mut event = Event::new(raw);
    let text = event.render_body();

    match syslog_loose::parse_message_with_year_exact(
        &text,
        |_| Local::now().year(),
        rfc.to_variant(),
    ) {
        Ok(msg) => apply_parsed(&mut event, msg),
        Err(e) => {
            event.meta.errors.push(format!("syslog parse failed: {e}"));
        }
    }

    event
        .resource
        .insert("receiver".to_string(), json!(receiver_name));
    event
}

fn apply_parsed(event: &mut Event, msg: Message<&str>) {
    event.body = Value::String(msg.msg.to_string());

    if let Some(ts) = msg.timestamp {
        event.timestamp = ts.with_timezone(&Utc);
    }

    if let Some(hostname) = msg.hostname {
        event.attributes.insert("hostname".to_string(), json!(hostname));
        event.resource.insert("host".to_string(), json!(hostname));
    }
    if let Some(appname) = msg.appname {
        event.attributes.insert("appname".to_string(), json!(appname));
    }
    if let Some(procid) = &msg.procid {
        event
            .attributes
            .insert("procid".to_string(), json!(procid.to_string()));
    }
    if let Some(msgid) = msg.msgid {
        event.attributes.insert("msgid".to_string(), json!(msgid));
    }
    if let Some(facility) = msg.facility {
        event
            .attributes
            .insert("syslog_facility".to_string(), json!(facility as i32));
        event
            .attributes
            .insert("facility_name".to_string(), json!(facility.as_str()));
    }
    if let Some(severity) = msg.severity {
        let number = severity as i32;
        event
            .attributes
            .insert("syslog_severity".to_string(), json!(number));
        event.severity = Some(Severity {
            number,
            text: severity.as_str().to_string(),
        });
    }

    if !msg.structured_data.is_empty() {
        let mut sd = Map::new();
        for elem in &msg.structured_data {
            let mut params = Map::new();
            for (k, v) in elem.params() {
                params.insert((*k).to_string(), json!(v));
            }
            sd.insert(elem.id.to_string(), Value::Object(params));
        }
        event
            .attributes
            .insert("structured_data".to_string(), Value::Object(sd));
    }

    event.resource.insert(
        "protocol".to_string(),
        json!(match msg.protocol {
            SyslogProtocol::RFC3164 => "rfc3164",
            SyslogProtocol::RFC5424(_) => "rfc5424",
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RfcMode;

    #[test]
    fn parses_rfc5424_message() {
        let raw = bytes::Bytes::from_static(
            br#"<34>1 2003-10-11T22:14:15.003Z mymachine.example.com su - ID47 [exampleSDID@32473 iut="3" eventSource="Application"] BOM'su root' failed for lonvick"#,
        );
        let event = parse_into_event(raw, RfcMode::Auto, "syslog/tcp");

        assert_eq!(event.attributes.get("hostname").unwrap(), "mymachine.example.com");
        assert_eq!(event.attributes.get("appname").unwrap(), "su");
        assert_eq!(event.attributes.get("msgid").unwrap(), "ID47");
        assert_eq!(event.attributes.get("syslog_severity").unwrap(), 2);
        assert_eq!(event.severity.as_ref().unwrap().text, "crit");
        assert!(event.attributes.get("structured_data").is_some());
        assert_eq!(event.timestamp.to_rfc3339(), "2003-10-11T22:14:15.003+00:00");
        assert!(event.meta.errors.is_empty());
    }

    #[test]
    fn parses_rfc3164_message() {
        let raw = bytes::Bytes::from_static(
            b"<34>Oct 11 22:14:15 mymachine su: 'su root' failed for lonvick on /dev/pts/8",
        );
        let event = parse_into_event(raw, RfcMode::Auto, "syslog/udp");

        assert_eq!(event.attributes.get("hostname").unwrap(), "mymachine");
        assert_eq!(event.attributes.get("appname").unwrap(), "su");
        assert_eq!(
            event.body.as_str().unwrap(),
            "'su root' failed for lonvick on /dev/pts/8"
        );
        assert!(event.meta.errors.is_empty());
    }
}
