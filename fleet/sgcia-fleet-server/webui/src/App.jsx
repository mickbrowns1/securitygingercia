import { Fragment, useState } from "react";
import { fetchJSON, sendJSON } from "./api.js";
import { useInterval } from "./useInterval.js";

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
  const [bulkTag, setBulkTag] = useState("");

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
                  <th>Host</th>
                  <th>Version</th>
                  <th>Health</th>
                  <th>Last seen</th>
                  <th>Tags</th>
                  <th>Drill down</th>
                  <th></th>
                  <th></th>
                </tr>
              </thead>
              <tbody>
                {agents.map((a) => (
                  <Fragment key={a.id}>
                    <tr className={!a.healthy ? "row-problem" : undefined}>
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
                    {pushTargetId === a.id && (
                      <tr>
                        <td colSpan={8}>
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
      </main>
    </>
  );
}
