import { Fragment, useState } from "react";
import { fetchJSON, sendJSON } from "./api.js";
import { useInterval } from "./useInterval.js";
import TopologyPanel from "./TopologyPanel.jsx";

const POLL_MS = 5000;

function formatAgo(iso) {
  if (!iso) return "never";
  const seconds = Math.max(0, Math.round((Date.now() - new Date(iso).getTime()) / 1000));
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  return `${hours}h ago`;
}

// The agent's local UI is loopback-only on its own host, by design -- this
// renders the SSH tunnel command as its own line, before the URL to open
// once that tunnel is up, rather than a single merged instruction.
function DrilldownHint({ agent }) {
  if (!agent.local_ui_addr) return <span className="drilldown-hint">no local UI address reported</span>;
  const port = agent.local_ui_addr.split(":").pop();
  const host = agent.hostname || agent.id;
  return (
    <span className="drilldown-hint">
      {`ssh -L ${port}:${agent.local_ui_addr} ${host}\n`}
      {`http://127.0.0.1:${port}/`}
    </span>
  );
}

function SummaryItem({ label, value }) {
  return (
    <div className="summary-item">
      <div className="label">{label}</div>
      <div className="value">{value}</div>
    </div>
  );
}

// TagsCell shows an agent's tags as small pills; clicking switches to a
// plain comma-separated text input (matching this UI's existing preference
// for plain text over dialogs, e.g. DrilldownHint above), saving via
// PUT /agents/{id}/tags on Enter or blur -- full-replace, not incremental.
function TagsCell({ agent, onSaved }) {
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(agent.tags.join(", "));
  const [error, setError] = useState("");

  function save() {
    const tags = value
      .split(",")
      .map((t) => t.trim())
      .filter(Boolean);
    sendJSON("PUT", `agents/${agent.id}/tags`, { tags })
      .then((res) => {
        setError("");
        setEditing(false);
        onSaved(agent.id, res.tags);
      })
      .catch((err) => setError(err.message));
  }

  if (editing) {
    return (
      <span>
        <input
          className="tag-input"
          autoFocus
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onBlur={save}
          onKeyDown={(e) => {
            if (e.key === "Enter") save();
            if (e.key === "Escape") setEditing(false);
          }}
        />
        {error && <div className="tag-error">{error}</div>}
      </span>
    );
  }

  return (
    <span className="tags-cell" onClick={() => setEditing(true)} title="Click to edit">
      {agent.tags.length === 0 ? (
        <span className="tag-empty">no tags</span>
      ) : (
        agent.tags.map((t) => (
          <span className="tag-pill" key={t}>
            {t}
          </span>
        ))
      )}
    </span>
  );
}

// sumAcrossAgents adds up one numeric field from one snapshot section
// (receivers/pipelines/exporters) across every component of that kind on
// every agent -- e.g. sumAcrossAgents(agents, "receivers", "events_in") is
// the fleet's true total ingest, matching the same events_in-not-
// events_out principle the per-agent Topology view already established
// (an exporter's own events_in can be inflated by fan-out, a receiver's
// can't).
function sumAcrossAgents(agents, section, field) {
  let total = 0;
  for (const a of agents) {
    const bucket = a.snapshot?.[section];
    if (!bucket) continue;
    for (const name in bucket) {
      total += bucket[name][field] || 0;
    }
  }
  return total;
}

// StatsPanel is a per-agent expandable row showing the same
// receivers/pipelines/exporters breakdown the agent's own Health view
// shows -- this data already arrives with every health report, it just
// wasn't surfaced anywhere in the fleet UI until now.
function StatsPanel({ agent }) {
  const snap = agent.snapshot;
  if (!snap) return <div className="empty-state">No snapshot reported yet.</div>;

  const receivers = Object.entries(snap.receivers || {});
  const pipelines = Object.entries(snap.pipelines || {});
  const exporters = Object.entries(snap.exporters || {});

  return (
    <div className="stats-panel">
      <div className="stats-group">
        <h3>Receivers</h3>
        <table className="data-table">
          <thead>
            <tr>
              <th>Receiver</th>
              <th className="numeric">Events in</th>
            </tr>
          </thead>
          <tbody>
            {receivers.map(([name, r]) => (
              <tr key={name}>
                <td>{name}</td>
                <td className="numeric">{r.events_in}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="stats-group">
        <h3>Pipelines</h3>
        <table className="data-table">
          <thead>
            <tr>
              <th>Pipeline</th>
              <th className="numeric">In</th>
              <th className="numeric">Out</th>
              <th className="numeric">Dropped</th>
              <th className="numeric">Parse errors</th>
            </tr>
          </thead>
          <tbody>
            {pipelines.map(([name, p]) => (
              <tr key={name} className={p.events_dropped || p.parse_errors ? "row-problem" : undefined}>
                <td>{name}</td>
                <td className="numeric">{p.events_in}</td>
                <td className="numeric">{p.events_out}</td>
                <td className="numeric">{p.events_dropped}</td>
                <td className="numeric">{p.parse_errors}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="stats-group">
        <h3>Exporters</h3>
        <table className="data-table">
          <thead>
            <tr>
              <th>Exporter</th>
              <th className="numeric">In</th>
              <th className="numeric">Sent</th>
              <th className="numeric">Failed</th>
            </tr>
          </thead>
          <tbody>
            {exporters.map(([name, e]) => (
              <tr key={name} className={e.batches_failed ? "row-problem" : undefined}>
                <td>
                  {name}
                  {e.last_error?.message && (
                    <span className="err-indicator" title={e.last_error.message}>
                      !
                    </span>
                  )}
                </td>
                <td className="numeric">{e.events_in}</td>
                <td className="numeric">{e.batches_sent}</td>
                <td className="numeric">{e.batches_failed}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

// PushConfigPanel is a per-agent expandable row -- a plain textarea +
// button, not a modal, matching this UI's existing style. Reused as-is
// for the bulk-by-tag form below (same shape, different endpoint).
function PushConfigPanel({ placeholder, onPush }) {
  const [text, setText] = useState("");
  const [status, setStatus] = useState(null);

  function push() {
    setStatus({ pending: true });
    onPush(text)
      .then((res) => setStatus({ pending: false, result: res }))
      .catch((err) => setStatus({ pending: false, error: err.message }));
  }

  return (
    <div className="push-panel">
      <textarea
        className="push-textarea"
        placeholder={placeholder}
        value={text}
        onChange={(e) => setText(e.target.value)}
        rows={8}
      />
      <button type="button" onClick={push} disabled={!text.trim() || status?.pending}>
        {status?.pending ? "Pushing..." : "Push"}
      </button>
      {status?.error && <div className="push-error">{status.error}</div>}
      {status?.result && (
        <pre className="push-result">{JSON.stringify(status.result, null, 2)}</pre>
      )}
    </div>
  );
}

export default function App() {
  const [agents, setAgents] = useState([]);
  const [connected, setConnected] = useState(true);
  const [connError, setConnError] = useState("");
  const [tagFilter, setTagFilter] = useState("");
  const [pushTargetId, setPushTargetId] = useState(null);
  const [statsTargetId, setStatsTargetId] = useState(null);
  const [bulkTag, setBulkTag] = useState("");
  const [selectedIds, setSelectedIds] = useState(() => new Set());

  useInterval(() => {
    const path = "agents" + (tagFilter ? `?tag=${encodeURIComponent(tagFilter)}` : "");
    fetchJSON(path)
      .then((list) => {
        setConnected(true);
        setAgents(list);
      })
      .catch((err) => {
        setConnected(false);
        setConnError(err.message);
      });
  }, POLL_MS);

  function applyTagsLocally(id, tags) {
    setAgents((prev) => prev.map((a) => (a.id === id ? { ...a, tags } : a)));
  }

  function toggleSelected(id) {
    setSelectedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  // Removes a stale/duplicate inventory row (e.g. left over from an agent
  // restart before its instance ID was persisted). If the agent is still
  // actually running, it just reappears on its next check-in -- this only
  // ever touches the fleet server's inventory, never the agent itself.
  function removeAgent(agent) {
    if (!confirm(`Remove ${agent.hostname || agent.id} from the fleet inventory? If it's still running, it will reappear on its next check-in.`)) return;
    sendJSON("DELETE", `agents/${agent.id}`)
      .then(() => setAgents((prev) => prev.filter((a) => a.id !== agent.id)))
      .catch((err) => alert(`Could not remove ${agent.hostname || agent.id}: ${err.message}`));
  }

  const healthyCount = agents.filter((a) => a.healthy).length;
  const unhealthyCount = agents.length - healthyCount;
  const totalEventsIn = sumAcrossAgents(agents, "receivers", "events_in");
  const totalDropped = sumAcrossAgents(agents, "pipelines", "events_dropped");
  const totalParseErrors = sumAcrossAgents(agents, "pipelines", "parse_errors");
  const totalBatchesFailed = sumAcrossAgents(agents, "exporters", "batches_failed");

  return (
    <>
      <header>
        <h1>Security Ginger Fleet</h1>
        <span className={"conn-status" + (connected ? "" : " down")}>
          {connected ? "connected" : `disconnected: ${connError}`}
        </span>
      </header>
      <main>
        <div className="panel">
          <h2>Overview</h2>
          <div className="summary-grid">
            <SummaryItem label="Agents" value={agents.length} />
            <SummaryItem label="Healthy" value={healthyCount} />
            <SummaryItem label="Unhealthy" value={unhealthyCount} />
            <SummaryItem label="Total events in" value={totalEventsIn} />
            <SummaryItem label="Total dropped" value={totalDropped} />
            <SummaryItem label="Total parse errors" value={totalParseErrors} />
            <SummaryItem label="Total batches failed" value={totalBatchesFailed} />
          </div>
        </div>

        <div className="panel">
          <h2>Agents</h2>
          <input
            className="tag-filter-input"
            placeholder="Filter by tag (e.g. env:prod)"
            value={tagFilter}
            onChange={(e) => setTagFilter(e.target.value)}
          />
          {agents.length === 0 ? (
            <div className="empty-state">
              {tagFilter
                ? "No agents carry that tag."
                : "No agents enrolled yet. Point an sgcia-otelcol instance's fleet config at this server to see it here."}
            </div>
          ) : (
            <table className="data-table">
              <thead>
                <tr>
                  <th title="Select for the Topology panel below">Compare</th>
                  <th>Host</th>
                  <th>Version</th>
                  <th>Health</th>
                  <th>Last seen</th>
                  <th>Tags</th>
                  <th>Drill down</th>
                  <th></th>
                  <th></th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {agents.map((a) => (
                  <Fragment key={a.id}>
                    <tr className={!a.healthy ? "row-problem" : undefined}>
                      <td>
                        <input type="checkbox" checked={selectedIds.has(a.id)} onChange={() => toggleSelected(a.id)} />
                      </td>
                      <td>{a.hostname || a.id}</td>
                      <td>{a.service_version || "-"}</td>
                      <td>
                        <span className={"health-dot " + (a.healthy ? "healthy" : "unhealthy")} />
                        {a.healthy ? "healthy" : "unhealthy"}
                        {!a.healthy && a.last_error && (
                          <span className="err-indicator" title={a.last_error}>
                            !
                          </span>
                        )}
                      </td>
                      <td>{formatAgo(a.last_seen)}</td>
                      <td>
                        <TagsCell agent={a} onSaved={applyTagsLocally} />
                      </td>
                      <td>
                        <DrilldownHint agent={a} />
                      </td>
                      <td>
                        <button type="button" onClick={() => setStatsTargetId(statsTargetId === a.id ? null : a.id)}>
                          {statsTargetId === a.id ? "Hide stats" : "Stats"}
                        </button>
                      </td>
                      <td>
                        <button type="button" onClick={() => setPushTargetId(pushTargetId === a.id ? null : a.id)}>
                          {pushTargetId === a.id ? "Cancel" : "Push config"}
                        </button>
                      </td>
                      <td>
                        <button type="button" className="danger-btn" onClick={() => removeAgent(a)}>
                          Remove
                        </button>
                      </td>
                    </tr>
                    {statsTargetId === a.id && (
                      <tr>
                        <td colSpan={10}>
                          <StatsPanel agent={a} />
                        </td>
                      </tr>
                    )}
                    {pushTargetId === a.id && (
                      <tr>
                        <td colSpan={10}>
                          <PushConfigPanel
                            placeholder={`Full config.yaml to push to ${a.hostname || a.id}...`}
                            onPush={(text) => sendJSON("POST", `agents/${a.id}/config`, { config_yaml: text })}
                          />
                        </td>
                      </tr>
                    )}
                  </Fragment>
                ))}
              </tbody>
            </table>
          )}
        </div>

        <div className="panel">
          <h2>Bulk push by tag</h2>
          <input
            className="tag-filter-input"
            placeholder="Tag to target (e.g. env:staging)"
            value={bulkTag}
            onChange={(e) => setBulkTag(e.target.value)}
          />
          {bulkTag.trim() && (
            <PushConfigPanel
              placeholder={`Full config.yaml to push to every agent tagged "${bulkTag.trim()}"...`}
              onPush={(text) =>
                sendJSON("POST", `agents/bulk/config?tag=${encodeURIComponent(bulkTag.trim())}`, {
                  config_yaml: text,
                })
              }
            />
          )}
        </div>

        <div className="panel">
          <h2>Topology</h2>
          {selectedIds.size === 0 ? (
            <div className="empty-state">Check one or more agents' "Compare" boxes above to see their topology diagrams here, stacked for comparison.</div>
          ) : (
            agents
              .filter((a) => selectedIds.has(a.id))
              .map((a) => <TopologyPanel key={a.id} agent={a} />)
          )}
        </div>
      </main>
    </>
  );
}
