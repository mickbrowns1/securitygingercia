import { forwardRef, useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import { fetchJSON } from "../api.js";
import { useInterval } from "../useInterval.js";

const LOGS_POLL_MS = 4000;

function severityClass(sev) {
  const s = (sev || "").toLowerCase();
  if (s.includes("err") || s.includes("fatal")) return "sev-error";
  if (s.includes("warn")) return "sev-warn";
  if (s.includes("info")) return "sev-info";
  if (s.includes("debug")) return "sev-debug";
  if (s.includes("trace")) return "sev-trace";
  return "sev-info";
}

function isErrorSeverity(sev) {
  const cls = severityClass(sev);
  return cls === "sev-error"; // covers both "err" and "fatal" per severityClass above
}

// Each attribute/resource pair renders as a clickable badge -- click one
// to filter the log view to only other events with that exact key=value
// (a lightweight correlation tool, e.g. "what else happened on this
// host/session?"). Exact match, not substring -- see the matching
// backend note in logbuffer.go's Snapshot for why that matters.
function AttrBadges({ entry, onPick }) {
  const pairs = [];
  const addFrom = (map) => {
    if (!map) return;
    Object.keys(map).forEach((k) => pairs.push([k, map[k]]));
  };
  addFrom(entry.attributes);
  addFrom(entry.resource);
  return pairs.map(([k, v]) => (
    <span
      key={`${k}=${v}`}
      className="corr-badge"
      title={`Click to filter to events with ${k}=${v}`}
      onClick={() => onPick(k, v)}
    >
      {k}={v}
    </span>
  ));
}

const LogsView = forwardRef(function LogsView({ active, onConnStatus }, ref) {
  const [entries, setEntries] = useState([]);
  const [severity, setSeverity] = useState("");
  const [corrFilter, setCorrFilter] = useState(null); // {key,value} | null
  const [autoRefresh, setAutoRefresh] = useState(true);
  const [knownSeverities, setKnownSeverities] = useState(() => new Set());
  const [lastQuery, setLastQuery] = useState("");
  const [highlightedIndex, setHighlightedIndex] = useState(-1);

  const queryInputRef = useRef(null);
  const rowRefs = useRef(new Map());

  const loadLogs = useCallback(() => {
    const q = queryInputRef.current ? queryInputRef.current.value.trim() : "";
    const params = new URLSearchParams();
    if (corrFilter) {
      params.set("attr_key", corrFilter.key);
      params.set("attr_value", corrFilter.value);
    } else if (q) {
      params.set("q", q);
    }
    if (severity) params.set("severity", severity);
    const path = "logs" + (params.toString() ? `?${params.toString()}` : "");
    fetchJSON(path)
      .then((fetched) => {
        onConnStatus(true);
        setEntries(fetched);
        setLastQuery(q);
        setKnownSeverities((prev) => {
          let changed = false;
          const next = new Set(prev);
          fetched.forEach((e) => {
            if (e.severity && !next.has(e.severity)) {
              next.add(e.severity);
              changed = true;
            }
          });
          return changed ? next : prev;
        });
      })
      .catch((err) => onConnStatus(false, err.message));
  }, [corrFilter, severity, onConnStatus]);

  useInterval(loadLogs, LOGS_POLL_MS, active && autoRefresh);

  // Re-fetch immediately on filter changes rather than waiting for the
  // next poll tick (matches the original's direct loadLogs() calls from
  // the severity <select>'s change handler and corr-badge clicks).
  useEffect(() => {
    loadLogs();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [severity, corrFilter]);

  function exportLogs() {
    const data = JSON.stringify(entries, null, 2);
    const blob = new Blob([data], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `sgcia-logs-${new Date().toISOString().replace(/[:.]/g, "-")}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }

  // Clears the buffer server-side (DELETE /logs) -- shared state, so
  // this affects every viewer of this collector's web UI, not just
  // whoever clicked it. Confirmed for exactly that reason.
  function clearLogBuffer() {
    if (!confirm("Clear the entire log buffer? This affects everyone viewing this collector, not just you.")) return;
    fetch("logs", { method: "DELETE" })
      .then((resp) => {
        if (!resp.ok) throw new Error(`${resp.status} ${resp.statusText}`);
        onConnStatus(true);
        loadLogs();
      })
      .catch((err) => onConnStatus(false, err.message));
  }

  function clearCorrFilter() {
    setCorrFilter(null);
  }

  const displayEntries = entries.slice().reverse();
  const errorIndices = displayEntries.reduce((acc, e, i) => {
    if (isErrorSeverity(e.severity)) acc.push(i);
    return acc;
  }, []);

  // Cycles through currently-displayed ERROR/FATAL rows (direction: +1
  // forward, -1 backward), highlighting and scrolling to each in turn.
  // Recomputed from the current entries on every call rather than
  // cached, since the next auto-refresh can replace the list at any
  // moment -- losing the current position across a refresh (falling
  // back to "start from the top") is an acceptable tradeoff for not
  // tracking stale rows.
  function jumpToError(direction) {
    if (errorIndices.length === 0) return;
    const currentPos = errorIndices.indexOf(highlightedIndex);
    const nextPos = currentPos === -1 ? (direction > 0 ? 0 : errorIndices.length - 1) : (currentPos + direction + errorIndices.length) % errorIndices.length;
    const nextIndex = errorIndices[nextPos];
    setHighlightedIndex(nextIndex);
    const row = rowRefs.current.get(nextIndex);
    if (row) row.scrollIntoView({ block: "nearest" });
  }

  useImperativeHandle(ref, () => ({
    focusSearch() {
      queryInputRef.current?.focus();
    },
    jumpToError,
    clearFilterAndSearch() {
      setCorrFilter(null);
      if (queryInputRef.current) queryInputRef.current.value = "";
      loadLogs();
    },
    hasActiveFilter() {
      return !!(corrFilter || (queryInputRef.current && queryInputRef.current.value));
    },
  }));

  const filterActive = !!(corrFilter || lastQuery || severity);

  return (
    <section className={"view" + (active ? " active" : "")} id="view-logs">
      <div className="panel">
        <div className="logs-toolbar">
          <input
            ref={queryInputRef}
            type="text"
            placeholder="Search body / attributes / resource..."
            onKeyDown={(ev) => {
              if (ev.key !== "Enter") return;
              setCorrFilter(null);
              loadLogs();
            }}
          />
          <select value={severity} onChange={(ev) => setSeverity(ev.target.value)}>
            <option value="">All severities</option>
            {Array.from(knownSeverities)
              .sort()
              .map((s) => (
                <option key={s} value={s}>
                  {s}
                </option>
              ))}
          </select>
          <button type="button" onClick={loadLogs}>
            Refresh
          </button>
          <button type="button" onClick={exportLogs}>
            Export
          </button>
          <button type="button" className="danger-btn" onClick={clearLogBuffer}>
            Clear buffer
          </button>
          <label className="autorefresh">
            <input type="checkbox" checked={autoRefresh} onChange={(ev) => setAutoRefresh(ev.target.checked)} /> Auto-refresh
          </label>
        </div>

        {corrFilter && (
          <div className="corr-chip">
            Filtering: <strong>{corrFilter.key}={corrFilter.value}</strong>{" "}
            <button type="button" onClick={clearCorrFilter}>
              &times;
            </button>
          </div>
        )}

        {displayEntries.length === 0 && (
          <div className="empty-state">
            {filterActive ? (
              "No matching events."
            ) : (
              <>
                No log records yet. Add a <code>logbuffer</code> exporter to a logs pipeline to see events here.
              </>
            )}
          </div>
        )}

        <table className="data-table logs-table">
          <thead>
            <tr>
              <th>Time</th>
              <th>Severity</th>
              <th>Body</th>
              <th>Attributes</th>
            </tr>
          </thead>
          <tbody>
            {displayEntries.map((e, i) => {
              const t = e.timestamp ? new Date(e.timestamp).toLocaleTimeString() : "";
              return (
                <tr
                  key={`${e.timestamp}-${i}`}
                  className={i === highlightedIndex ? "nav-highlight" : undefined}
                  ref={(el) => {
                    if (el) rowRefs.current.set(i, el);
                    else rowRefs.current.delete(i);
                  }}
                >
                  <td>{t}</td>
                  <td>
                    <span className={`sev-badge ${severityClass(e.severity)}`}>{e.severity || ""}</span>
                  </td>
                  <td className="body-cell">{e.body}</td>
                  <td className="attrs-cell">
                    <AttrBadges
                      entry={e}
                      onPick={(key, value) => {
                        setCorrFilter({ key, value });
                        if (queryInputRef.current) queryInputRef.current.value = "";
                      }}
                    />
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </section>
  );
});

export default LogsView;
