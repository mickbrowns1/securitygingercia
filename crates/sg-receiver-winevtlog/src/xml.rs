use sg_core::Event;

/// Minimal, dependency-free extraction of a handful of well-known fields
/// out of the XML that `EvtRender(..., EvtRenderEventXml, ...)` produces.
/// Deliberately not a general XML parser: Windows Event Log XML has a
/// fixed, well-documented shape, and pulling in a full parser for five
/// fields would be more scope than this needs.
pub fn extract_tag_text(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].to_string())
}

pub fn extract_attr(xml: &str, tag: &str, attr: &str) -> Option<String> {
    let tag_start = xml.find(&format!("<{tag} "))?;
    let rest = &xml[tag_start..];
    let tag_end = rest.find('>')?;
    let tag_slice = &rest[..tag_end];
    let attr_pat = format!("{attr}=\"");
    let attr_start = tag_slice.find(&attr_pat)? + attr_pat.len();
    let attr_end = tag_slice[attr_start..].find('"')? + attr_start;
    Some(tag_slice[attr_start..attr_end].to_string())
}

/// Windows Event Log `Level` values (winmeta.xml): 0=LogAlways,
/// 1=Critical, 2=Error, 3=Warning, 4=Informational, 5=Verbose. Distinct
/// scale from syslog's 0-7 severity, so it's mapped into
/// `event.severity` with its own small table rather than reused.
fn level_text(level: i32) -> &'static str {
    match level {
        1 => "critical",
        2 => "error",
        3 => "warning",
        4 => "informational",
        5 => "verbose",
        _ => "log_always",
    }
}

/// Builds an `Event` from one rendered Event XML document.
pub fn parse_event_xml(xml: String, receiver_name: &str) -> Event {
    let mut event = Event::new(bytes::Bytes::from(xml.clone().into_bytes()));
    event.body = serde_json::Value::String(xml.clone());

    if let Some(provider) = extract_attr(&xml, "Provider", "Name") {
        event
            .attributes
            .insert("provider".to_string(), serde_json::json!(provider));
    }
    if let Some(event_id) = extract_tag_text(&xml, "EventID") {
        event
            .attributes
            .insert("event_id".to_string(), serde_json::json!(event_id));
    }
    if let Some(record_id) = extract_tag_text(&xml, "EventRecordID") {
        event
            .attributes
            .insert("event_record_id".to_string(), serde_json::json!(record_id));
    }
    if let Some(channel) = extract_tag_text(&xml, "Channel") {
        event
            .attributes
            .insert("channel".to_string(), serde_json::json!(channel));
    }
    if let Some(computer) = extract_tag_text(&xml, "Computer") {
        event.resource.insert("host".to_string(), serde_json::json!(&computer));
        event
            .attributes
            .insert("computer".to_string(), serde_json::json!(computer));
    }
    if let Some(level) = extract_tag_text(&xml, "Level").and_then(|s| s.parse::<i32>().ok()) {
        event
            .attributes
            .insert("level".to_string(), serde_json::json!(level));
        event.severity = Some(sg_core::Severity {
            number: level,
            text: level_text(level).to_string(),
        });
    }
    if let Some(time) = extract_attr(&xml, "TimeCreated", "SystemTime") {
        if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(&time) {
            event.timestamp = ts.with_timezone(&chrono::Utc);
        }
        event
            .attributes
            .insert("time_created".to_string(), serde_json::json!(time));
    }

    event
        .resource
        .insert("receiver".to_string(), serde_json::json!(receiver_name));
    event
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"<Event xmlns="http://schemas.microsoft.com/win/2004/08/events/event">
  <System>
    <Provider Name="Microsoft-Windows-Security-Auditing" Guid="{54849625-5478-4994-a5ba-3e3b0328c30d}"/>
    <EventID>4624</EventID>
    <Version>2</Version>
    <Level>0</Level>
    <Task>12544</Task>
    <Opcode>0</Opcode>
    <Keywords>0x8020000000000000</Keywords>
    <TimeCreated SystemTime="2026-07-28T15:00:00.123456700Z"/>
    <EventRecordID>123456</EventRecordID>
    <Correlation/>
    <Execution ProcessID="600" ThreadID="612"/>
    <Channel>Security</Channel>
    <Computer>WIN-HOST01</Computer>
    <Security/>
  </System>
  <EventData>
    <Data Name="SubjectUserSid">S-1-5-18</Data>
    <Data Name="TargetUserName">jdoe</Data>
  </EventData>
</Event>"#;

    #[test]
    fn extracts_known_fields_from_rendered_xml() {
        let event = parse_event_xml(SAMPLE.to_string(), "windows_eventlog/security");
        assert_eq!(
            event.attributes.get("provider").unwrap(),
            "Microsoft-Windows-Security-Auditing"
        );
        assert_eq!(event.attributes.get("event_id").unwrap(), "4624");
        assert_eq!(event.attributes.get("event_record_id").unwrap(), "123456");
        assert_eq!(event.attributes.get("channel").unwrap(), "Security");
        assert_eq!(event.resource.get("host").unwrap(), "WIN-HOST01");
        assert_eq!(event.timestamp.to_rfc3339(), "2026-07-28T15:00:00.123456700+00:00");
        assert!(event.body.as_str().unwrap().contains("4624"));
    }

    #[test]
    fn missing_fields_are_simply_absent_not_an_error() {
        let event = parse_event_xml("<Event></Event>".to_string(), "windows_eventlog/security");
        assert!(event.attributes.get("provider").is_none());
        assert!(event.attributes.get("event_id").is_none());
    }
}
