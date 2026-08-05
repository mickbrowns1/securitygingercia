import { useState } from "react";
import { fetchJSON } from "./api.js";
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

export default function App() {
  const [agents, setAgents] = useState([]);
  const [connected, setConnected] = useState(true);
  const [connError, setConnError] = useState("");

  useInterval(() => {
    fetchJSON("agents")
      .then((list) => {
        setConnected(true);
        setAgents(list);
      })
      .catch((err) => {
        setConnected(false);
        setConnError(err.message);
      });
  }, POLL_MS);

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
          {agents.length === 0 ? (
            <div className="empty-state">No agents enrolled yet. Point an sgcia-otelcol instance's fleet config at this server to see it here.</div>
          ) : (
            <table className="data-table">
              <thead>
                <tr>
                  <th>Host</th>
                  <th>Version</th>
                  <th>Health</th>
                  <th>Last seen</th>
                  <th>Drill down</th>
                </tr>
              </thead>
              <tbody>
                {agents.map((a) => (
                  <tr key={a.id} className={!a.healthy ? "row-problem" : undefined}>
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
                      <DrilldownHint agent={a} />
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </main>
    </>
  );
}
