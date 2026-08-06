import { Fragment, useEffect, useRef, useState } from "react";
import { fetchJSON, sendJSON, uploadFile } from "./api.js";
import { useInterval } from "./useInterval.js";
import TopologyPanel from "./TopologyPanel.jsx";

const POLL_MS = 5000;

// This project only ever manages one binary via fleet package rollout
// (sgcia-otelcol) -- the fleet server's own schema is name-generic, but
// the UI doesn't need to expose a name field an operator would only ever
// fill in one way.
const PACKAGE_NAME = "sgcia-otelcol";

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

// description is optional -- when given, an "info-indicator" badge sits
// next to the label with the full explanation as a hover tooltip, the
// same affordance the per-agent badges below use (see err-indicator/
// warn-indicator) rather than inventing a new pattern for it.
function SummaryItem({ label, value, description }) {
  return (
    <div className="summary-item">
      <div className="label">
        {label}
        {description && (
          <span className="info-indicator" title={description}>
            i
          </span>
        )}
      </div>
      <div className="value">{value}</div>
    </div>
  );
}

// TagsCell shows an agent's tags as small pills; clicking switches to a
// plain comma-separated text input (matching this UI's existing preference
// for plain text over dialogs, e.g. DrilldownHint above), saving via
// PUT /agents/{id}/tags on Enter or blur -- full-replace, not incremental.
// activeFilters is every currently-typed tag-input value across the page
// (the Agents "Filter by tag" box, and the two bulk-push-by-tag boxes) --
// any pill that matches one of them exactly (the same exact-match
// semantics the server itself uses to filter/target by tag) lights up
// green, giving instant feedback on what you're about to filter or target
// before the next poll cycle actually re-fetches the filtered list.
function TagsCell({ agent, onSaved, activeFilters }) {
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
          <span className={"tag-pill" + (activeFilters.includes(t) ? " tag-pill-match" : "")} key={t}>
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
  const process = snap.process;

  return (
    <div className="stats-panel">
      {process && (
        <div className="stats-group">
          <h3>Process</h3>
          <table className="data-table">
            <tbody>
              <tr>
                <td>CPU time</td>
                <td className="numeric">{process.cpu_seconds.toFixed(1)}s</td>
              </tr>
              <tr>
                <td>Memory (RSS)</td>
                <td className="numeric">{(process.memory_rss_bytes / 1048576).toFixed(1)} MB</td>
              </tr>
              <tr>
                <td>Heap</td>
                <td className="numeric">{(process.heap_alloc_bytes / 1048576).toFixed(1)} MB</td>
              </tr>
            </tbody>
          </table>
        </div>
      )}
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

// UploadPackagePanel is a file input + version field + button -- this
// project's only binary upload path, POSTing the raw file body to
// POST /packages/{name}?version=... (see api.js's uploadFile). The fleet
// server computes and records the content hash itself; nothing here needs
// to know or send it.
function UploadPackagePanel({ onUploaded }) {
  const [file, setFile] = useState(null);
  const [version, setVersion] = useState("");
  const [status, setStatus] = useState(null);

  function upload() {
    setStatus({ pending: true });
    uploadFile(`packages/${PACKAGE_NAME}?version=${encodeURIComponent(version.trim())}`, file)
      .then((res) => {
        setStatus({ pending: false, result: res });
        setFile(null);
        setVersion("");
        onUploaded();
      })
      .catch((err) => setStatus({ pending: false, error: err.message }));
  }

  return (
    <div className="push-panel">
      <input type="file" onChange={(e) => setFile(e.target.files[0] || null)} />
      <input
        className="tag-filter-input"
        placeholder="Version (e.g. 0.1.1)"
        value={version}
        onChange={(e) => setVersion(e.target.value)}
      />
      <button type="button" onClick={upload} disabled={!file || !version.trim() || status?.pending}>
        {status?.pending ? "Uploading..." : "Upload"}
      </button>
      {status?.error && <div className="push-error">{status.error}</div>}
      {status?.result && <pre className="push-result">{JSON.stringify(status.result, null, 2)}</pre>}
    </div>
  );
}

// PackageList shows every uploaded version, newest first (as the fleet
// server already returns them) -- what an operator checks before pushing
// to confirm a build actually landed.
function PackageList({ packages }) {
  if (packages.length === 0) return <div className="empty-state">No packages uploaded yet.</div>;
  return (
    <table className="data-table">
      <thead>
        <tr>
          <th>Version</th>
          <th>Hash</th>
          <th>Uploaded</th>
        </tr>
      </thead>
      <tbody>
        {packages.map((p) => (
          <tr key={p.version}>
            <td>{p.version}</td>
            <td className="drilldown-hint">{p.hash.slice(0, 12)}...</td>
            <td>{formatAgo(p.uploaded_at)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// PushPackagePanel is a version picker (populated from already-uploaded
// packages) + push button -- unlike PushConfigPanel there's no free-text
// content to paste; the fleet server already has the bytes and hash on
// disk from upload, so pushing only ever needs to name a version. Reused
// as-is for the bulk-by-tag form below.
function PushPackagePanel({ packages, onPush }) {
  const [version, setVersion] = useState("");
  const [status, setStatus] = useState(null);

  function push() {
    setStatus({ pending: true });
    onPush(version)
      .then((res) => setStatus({ pending: false, result: res }))
      .catch((err) => setStatus({ pending: false, error: err.message }));
  }

  return (
    <div className="push-panel">
      <select value={version} onChange={(e) => setVersion(e.target.value)}>
        <option value="">Select a version...</option>
        {packages.map((p) => (
          <option key={p.version} value={p.version}>
            {p.version}
          </option>
        ))}
      </select>
      <button type="button" onClick={push} disabled={!version || status?.pending}>
        {status?.pending ? "Pushing..." : "Push"}
      </button>
      {status?.error && <div className="push-error">{status.error}</div>}
      {status?.result && <pre className="push-result">{JSON.stringify(status.result, null, 2)}</pre>}
    </div>
  );
}

// ActionsMenu consolidates what used to be five separate unlabeled table
// columns (Stats/Push config/Push package/Rollback pkg/Remove, repeated
// identically on every row) into a single dropdown -- that's what made
// the Agents table cramped and hard to scan. Closes on an outside click
// or immediately after choosing an item.
function ActionsMenu({ agent, statsOpen, pushConfigOpen, pushPackageOpen, onToggleStats, onTogglePushConfig, onTogglePushPackage, onRollbackPackage, onRemove }) {
  const [open, setOpen] = useState(false);
  const rootRef = useRef(null);

  useEffect(() => {
    if (!open) return;
    function onClickOutside(e) {
      if (rootRef.current && !rootRef.current.contains(e.target)) setOpen(false);
    }
    document.addEventListener("mousedown", onClickOutside);
    return () => document.removeEventListener("mousedown", onClickOutside);
  }, [open]);

  function choose(action) {
    setOpen(false);
    action();
  }

  return (
    <div className="actions-menu" ref={rootRef}>
      <button type="button" onClick={() => setOpen((o) => !o)}>
        Actions {open ? "▲" : "▼"}
      </button>
      {open && (
        <div className="actions-menu-panel">
          <button type="button" className="actions-menu-item" onClick={() => choose(onToggleStats)}>
            {statsOpen ? "Hide stats" : "Stats"}
          </button>
          <button type="button" className="actions-menu-item" onClick={() => choose(onTogglePushConfig)}>
            {pushConfigOpen ? "Cancel New Config" : "Push New Config"}
          </button>
          <button type="button" className="actions-menu-item" onClick={() => choose(onTogglePushPackage)}>
            {pushPackageOpen ? "Cancel Agent Update" : "Push Agent Update"}
          </button>
          <button
            type="button"
            className="actions-menu-item"
            disabled={!agent.last_known_good_package_version}
            onClick={() => choose(onRollbackPackage)}
          >
            {agent.last_known_good_package_version ? `Roll back to ${agent.last_known_good_package_version}` : "Rollback (none recorded)"}
          </button>
          <button type="button" className="actions-menu-item danger" onClick={() => choose(onRemove)}>
            Remove
          </button>
        </div>
      )}
    </div>
  );
}

export default function App() {
  const [agents, setAgents] = useState([]);
  const [packages, setPackages] = useState([]);
  const [connected, setConnected] = useState(true);
  const [connError, setConnError] = useState("");
  const [tagFilter, setTagFilter] = useState("");
  const [pushTargetId, setPushTargetId] = useState(null);
  const [pkgPushTargetId, setPkgPushTargetId] = useState(null);
  const [statsTargetId, setStatsTargetId] = useState(null);
  const [bulkTag, setBulkTag] = useState("");
  const [bulkPkgTag, setBulkPkgTag] = useState("");
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

  function refreshPackages() {
    fetchJSON("packages")
      .then((list) => setPackages(list.filter((p) => p.name === PACKAGE_NAME)))
      .catch(() => {
        // Non-critical panel -- the agents polling above already surfaces
        // connectivity problems; a failed package list refresh just leaves
        // the previous list showing until the next successful poll.
      });
  }

  useInterval(refreshPackages, POLL_MS);

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

  // Re-pushes an agent's last-known-good package -- a one-click action
  // (no form) since there's nothing to fill in, matching how config
  // rollback works via the API today.
  function rollbackPackage(agent) {
    if (!agent.last_known_good_package_version) return;
    if (!confirm(`Roll ${agent.hostname || agent.id} back to package version ${agent.last_known_good_package_version}?`)) return;
    sendJSON("POST", `agents/${agent.id}/package/rollback`)
      .catch((err) => alert(`Could not roll back ${agent.hostname || agent.id}: ${err.message}`));
  }

  const healthyCount = agents.filter((a) => a.healthy).length;
  const unhealthyCount = agents.length - healthyCount;
  const totalEventsIn = sumAcrossAgents(agents, "receivers", "events_in");
  const totalDropped = sumAcrossAgents(agents, "pipelines", "events_dropped");
  const totalParseErrors = sumAcrossAgents(agents, "pipelines", "parse_errors");
  const totalBatchesFailed = sumAcrossAgents(agents, "exporters", "batches_failed");
  const flappingCount = agents.filter((a) => a.flapping).length;
  const driftedCount = agents.filter((a) => a.config_drifted).length;
  const activeTagFilters = [tagFilter, bulkTag, bulkPkgTag].map((s) => s.trim()).filter(Boolean);

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
            <SummaryItem
              label="Flapping"
              value={flappingCount}
              description="Agents that have restarted 3 or more times in the last 10 minutes -- a sign of a crash loop or a bad config/package push."
            />
            <SummaryItem
              label="Config drift"
              value={driftedCount}
              description="Agents whose actually-running config no longer matches what the fleet last pushed -- usually means someone edited the file directly on the box."
            />
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
                  <th>Actions</th>
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
                      <td>
                        {a.service_version || "-"}
                        {a.config_drifted && (
                          <span className="warn-indicator" title="Running config doesn't match what the fleet last pushed">
                            &ne;
                          </span>
                        )}
                      </td>
                      <td>
                        <span className={"health-dot " + (a.healthy ? "healthy" : "unhealthy")} />
                        {a.healthy ? "healthy" : "unhealthy"}
                        {!a.healthy && a.last_error && (
                          <span className="err-indicator" title={a.last_error}>
                            !
                          </span>
                        )}
                        {a.flapping && (
                          <span
                            className="warn-indicator"
                            title={`Restarted ${a.restart_count_recent} times in the last 10 minutes`}
                          >
                            &#10227;
                          </span>
                        )}
                      </td>
                      <td>{formatAgo(a.last_seen)}</td>
                      <td>
                        <TagsCell agent={a} onSaved={applyTagsLocally} activeFilters={activeTagFilters} />
                      </td>
                      <td>
                        <DrilldownHint agent={a} />
                      </td>
                      <td>
                        <ActionsMenu
                          agent={a}
                          statsOpen={statsTargetId === a.id}
                          pushConfigOpen={pushTargetId === a.id}
                          pushPackageOpen={pkgPushTargetId === a.id}
                          onToggleStats={() => setStatsTargetId(statsTargetId === a.id ? null : a.id)}
                          onTogglePushConfig={() => setPushTargetId(pushTargetId === a.id ? null : a.id)}
                          onTogglePushPackage={() => setPkgPushTargetId(pkgPushTargetId === a.id ? null : a.id)}
                          onRollbackPackage={() => rollbackPackage(a)}
                          onRemove={() => removeAgent(a)}
                        />
                      </td>
                    </tr>
                    {statsTargetId === a.id && (
                      <tr>
                        <td colSpan={8}>
                          <StatsPanel agent={a} />
                        </td>
                      </tr>
                    )}
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
                    {pkgPushTargetId === a.id && (
                      <tr>
                        <td colSpan={8}>
                          <PushPackagePanel
                            packages={packages}
                            onPush={(version) => sendJSON("POST", `agents/${a.id}/package`, { name: PACKAGE_NAME, version })}
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
          <h2>Topology</h2>
          {selectedIds.size === 0 ? (
            <div className="empty-state">Check one or more agents' "Compare" boxes above to see their topology diagrams here, stacked for comparison.</div>
          ) : (
            agents
              .filter((a) => selectedIds.has(a.id))
              .map((a) => <TopologyPanel key={a.id} agent={a} />)
          )}
        </div>

        <div className="panel">
          <h2>Agent &amp; Configuration Maintenance</h2>
          <div className="stats-panel">
            <div className="stats-group">
              <h3>Push New Config by tag</h3>
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
            <div className="stats-group">
              <h3>Push Agent Update by tag</h3>
              <input
                className="tag-filter-input"
                placeholder="Tag to target (e.g. env:staging)"
                value={bulkPkgTag}
                onChange={(e) => setBulkPkgTag(e.target.value)}
              />
              {bulkPkgTag.trim() && (
                <PushPackagePanel
                  packages={packages}
                  onPush={(version) =>
                    sendJSON("POST", `agents/bulk/package?tag=${encodeURIComponent(bulkPkgTag.trim())}`, {
                      name: PACKAGE_NAME,
                      version,
                    })
                  }
                />
              )}
            </div>
            <div className="stats-group">
              <h3>Packages</h3>
              <UploadPackagePanel onUploaded={refreshPackages} />
              <PackageList packages={packages} />
            </div>
          </div>
        </div>
      </main>
    </>
  );
}
