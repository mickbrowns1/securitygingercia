package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"strings"
	"sync"
	"time"
)

// LogEntry mirrors the logbuffer exporter's own POST /internal/logs wire
// shape -- a plain JSON contract, not a shared Go type, so that exporter
// has no local cross-module dependency on this extension at all.
type LogEntry struct {
	Timestamp  time.Time         `json:"timestamp"`
	Severity   string            `json:"severity"`
	Body       string            `json:"body"`
	Attributes map[string]string `json:"attributes,omitempty"`
	Resource   map[string]string `json:"resource,omitempty"`
}

// logBufferCapacity is intentionally small and fixed (not a config
// field, at least for now) -- this is a rolling "what just happened"
// window for the web UI's log viewer, not a real log store.
const logBufferCapacity = 500

// logBuffer is a fixed-capacity, thread-safe ring buffer of the most
// recently ingested log records. Bounded and in-memory only -- nothing
// here is persisted or retained across a restart. Fed by POST
// /internal/logs (from a pipeline's logbuffer exporter, if it has one)
// and served back out at GET /logs.
type logBuffer struct {
	mu      sync.Mutex
	entries []LogEntry // fixed-length ring; zero-valued until first wrap
	next    int
	full    bool
}

func newLogBuffer() *logBuffer {
	return &logBuffer{entries: make([]LogEntry, logBufferCapacity)}
}

func (b *logBuffer) Push(entries []LogEntry) {
	b.mu.Lock()
	defer b.mu.Unlock()
	for _, e := range entries {
		b.entries[b.next] = e
		b.next = (b.next + 1) % logBufferCapacity
		if b.next == 0 {
			b.full = true
		}
	}
}

// Clear discards every buffered entry -- backs the web UI's "Clear
// buffer" action (DELETE /logs). Shared, server-side state: this
// affects every viewer of the web UI, not just whoever clicked it.
func (b *logBuffer) Clear() {
	b.mu.Lock()
	defer b.mu.Unlock()
	b.entries = make([]LogEntry, logBufferCapacity)
	b.next = 0
	b.full = false
}

// Snapshot returns entries in chronological order (oldest first),
// optionally filtered by any combination of: a case-insensitive
// substring match against the body/attributes/resource (query), an
// exact case-insensitive severity match, and an exact (not substring)
// match of attrKey against either the attributes or resource map --
// this last one is what powers the web UI's click-a-badge-to-correlate
// feature, where "exact" matters: a substring match on e.g. a numeric
// session id would pick up unrelated events that just happen to contain
// the same digits somewhere in their body text.
func (b *logBuffer) Snapshot(query, severity, attrKey, attrValue string) []LogEntry {
	b.mu.Lock()
	ordered := make([]LogEntry, 0, logBufferCapacity)
	if b.full {
		ordered = append(ordered, b.entries[b.next:]...)
		ordered = append(ordered, b.entries[:b.next]...)
	} else {
		ordered = append(ordered, b.entries[:b.next]...)
	}
	b.mu.Unlock()

	if query == "" && severity == "" && attrKey == "" {
		return ordered
	}
	query = strings.ToLower(query)
	out := make([]LogEntry, 0, len(ordered))
	for _, e := range ordered {
		if severity != "" && !strings.EqualFold(e.Severity, severity) {
			continue
		}
		if attrKey != "" && e.Attributes[attrKey] != attrValue && e.Resource[attrKey] != attrValue {
			continue
		}
		if query != "" && !strings.Contains(strings.ToLower(e.Body), query) &&
			!mapContains(e.Attributes, query) && !mapContains(e.Resource, query) {
			continue
		}
		out = append(out, e)
	}
	return out
}

func mapContains(m map[string]string, query string) bool {
	for k, v := range m {
		if strings.Contains(strings.ToLower(k), query) || strings.Contains(strings.ToLower(v), query) {
			return true
		}
	}
	return false
}
