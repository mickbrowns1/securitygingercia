package statuscfgextension

import (
	"testing"
	"time"
)

func entry(body, severity string, attrs, resource map[string]string) LogEntry {
	return LogEntry{Timestamp: time.Now(), Severity: severity, Body: body, Attributes: attrs, Resource: resource}
}

func TestLogBuffer_SnapshotWithNoFiltersReturnsEverythingInOrder(t *testing.T) {
	b := newLogBuffer()
	b.Push([]LogEntry{entry("first", "INFO", nil, nil), entry("second", "INFO", nil, nil)})

	got := b.Snapshot("", "", "", "")
	if len(got) != 2 || got[0].Body != "first" || got[1].Body != "second" {
		t.Fatalf("got %+v, want [first, second] in order", got)
	}
}

func TestLogBuffer_SnapshotFiltersBySeverityCaseInsensitively(t *testing.T) {
	b := newLogBuffer()
	b.Push([]LogEntry{entry("a", "ERROR", nil, nil), entry("b", "INFO", nil, nil)})

	got := b.Snapshot("", "error", "", "")
	if len(got) != 1 || got[0].Body != "a" {
		t.Fatalf("got %+v, want just the ERROR entry", got)
	}
}

func TestLogBuffer_SnapshotFiltersByQuerySubstringAcrossBodyAndAttributes(t *testing.T) {
	b := newLogBuffer()
	b.Push([]LogEntry{
		entry("connection refused", "INFO", nil, nil),
		entry("unrelated", "INFO", map[string]string{"sourcetype": "cisco_asa"}, nil),
		entry("also unrelated", "INFO", nil, nil),
	})

	got := b.Snapshot("cisco", "", "", "")
	if len(got) != 1 || got[0].Body != "unrelated" {
		t.Fatalf("got %+v, want just the entry with a matching attribute", got)
	}
}

func TestLogBuffer_SnapshotAttrFilterMatchesExactAttributeValue(t *testing.T) {
	b := newLogBuffer()
	b.Push([]LogEntry{
		entry("a", "INFO", map[string]string{"sourcetype": "cisco_asa"}, nil),
		entry("b", "INFO", map[string]string{"sourcetype": "myapp"}, nil),
	})

	got := b.Snapshot("", "", "sourcetype", "cisco_asa")
	if len(got) != 1 || got[0].Body != "a" {
		t.Fatalf("got %+v, want just the entry with sourcetype=cisco_asa", got)
	}
}

func TestLogBuffer_SnapshotAttrFilterAlsoMatchesResourceMap(t *testing.T) {
	b := newLogBuffer()
	b.Push([]LogEntry{
		entry("a", "INFO", nil, map[string]string{"host.name": "web-01"}),
		entry("b", "INFO", nil, map[string]string{"host.name": "web-02"}),
	})

	got := b.Snapshot("", "", "host.name", "web-01")
	if len(got) != 1 || got[0].Body != "a" {
		t.Fatalf("got %+v, want just the entry with host.name=web-01", got)
	}
}

func TestLogBuffer_SnapshotAttrFilterIsExactNotSubstring(t *testing.T) {
	b := newLogBuffer()
	b.Push([]LogEntry{
		entry("a", "INFO", map[string]string{"session_id": "42"}, nil),
		entry("b about session 42 in passing", "INFO", nil, nil),
	})

	got := b.Snapshot("", "", "session_id", "42")
	if len(got) != 1 || got[0].Body != "a" {
		t.Fatalf("got %+v, want only the entry with an exact session_id=42 attribute, not the one that merely mentions 42 in its body", got)
	}
}

func TestLogBuffer_SnapshotCombinesAttrFilterWithSeverity(t *testing.T) {
	b := newLogBuffer()
	b.Push([]LogEntry{
		entry("a", "ERROR", map[string]string{"host": "web-01"}, nil),
		entry("b", "INFO", map[string]string{"host": "web-01"}, nil),
	})

	got := b.Snapshot("", "error", "host", "web-01")
	if len(got) != 1 || got[0].Body != "a" {
		t.Fatalf("got %+v, want only the ERROR entry with host=web-01", got)
	}
}
