import { useState } from "react";
import { fetchJSON } from "../api.js";
import { useInterval } from "../useInterval.js";

const HEALTH_POLL_MS = 5000;

function formatUptime(seconds) {
  seconds = seconds || 0;
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = seconds % 60;
  if (h > 0) return `${h}h ${m}m`;
  if (m > 0) return `${m}m ${s}s`;
  return `${s}s`;
}

// Renders an explicit visible bar (not just a background tint -- that
// was tried first and was too subtle to notice in practice) showing
// this row's count relative to the busiest row in the same table, with
// the number right above it.
function VolCell({ value, max }) {
  const pct = max > 0 ? Math.round((value / max) * 100) : 0;
  return (
    <td className="numeric vol-cell">
      <span className="vol-number">{value}</span>
      <span className="vol-track">
        <span className="vol-fill" style={{ width: `${pct}%` }} />
      </span>
    </td>
  );
}

// last_error (message + timestamp) already comes back from /status but
// previously had nowhere to show up in the UI at all -- the only way to
// see *why* an exporter was failing was journalctl on the box itself.
function ErrorIndicator({ lastError }) {
  if (!lastError || !lastError.message) return null;
  const when = lastError.at ? new Date(lastError.at).toLocaleString() : "unknown time";
  return (
    <span className="err-indicator" title={`${when}: ${lastError.message}`}>
      !
    </span>
  );
}

function EmptyRow({ cols }) {
  return (
    <tr>
      <td colSpan={cols} style={{ color: "var(--muted)" }}>
        no data yet
      </td>
    </tr>
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

export default function HealthView({ active, onConnStatus }) {
  const [snapshot, setSnapshot] = useState(null);

  useInterval(
    () => {
      fetchJSON("status")
        .then((snap) => {
          onConnStatus(true);
          setSnapshot(snap);
        })
        .catch((err) => onConnStatus(false, err.message));
    },
    HEALTH_POLL_MS,
    active
  );

  const pipelines = snapshot?.pipelines || {};
  const receivers = snapshot?.receivers || {};
  const exporters = snapshot?.exporters || {};

  const pipelineNames = Object.keys(pipelines).sort();
  const maxPipelineIn = pipelineNames.reduce((m, n) => Math.max(m, pipelines[n].events_in || 0), 0);

  const receiverIds = Object.keys(receivers).sort();
  const maxReceiverIn = receiverIds.reduce((m, id) => Math.max(m, receivers[id].events_in || 0), 0);

  const exporterIds = Object.keys(exporters).sort();
  const maxExporterIn = exporterIds.reduce((m, id) => Math.max(m, exporters[id].events_in || 0), 0);

  return (
    <section className={"view" + (active ? " active" : "")} id="view-health">
      <div className="panel">
        <h2>Overview</h2>
        <div className="summary-grid">
          <SummaryItem label="Uptime" value={formatUptime(snapshot?.uptime_seconds)} />
          <SummaryItem label="Receivers" value={receiverIds.length} />
          <SummaryItem label="Pipelines" value={pipelineNames.length} />
          <SummaryItem label="Exporters" value={exporterIds.length} />
        </div>
      </div>

      <div className="panel">
        <h2>Pipelines</h2>
        <table className="data-table">
          <thead>
            <tr>
              <th>Pipeline</th>
              <th className="numeric">Events in</th>
              <th className="numeric">Events out</th>
              <th className="numeric">Dropped</th>
            </tr>
          </thead>
          <tbody>
            {pipelineNames.length === 0 ? (
              <EmptyRow cols={4} />
            ) : (
              pipelineNames.map((name) => {
                const p = pipelines[name];
                const problem = (p.events_dropped || 0) > 0 || (p.parse_errors || 0) > 0;
                return (
                  <tr key={name} className={problem ? "row-problem" : undefined}>
                    <td>{name}</td>
                    <VolCell value={p.events_in} max={maxPipelineIn} />
                    <td className="numeric">{p.events_out}</td>
                    <td className="numeric">{p.events_dropped}</td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>

      <div className="panel-row">
        <div className="panel">
          <h2>Receivers</h2>
          <table className="data-table">
            <thead>
              <tr>
                <th>Receiver</th>
                <th className="numeric">Events in</th>
              </tr>
            </thead>
            <tbody>
              {receiverIds.length === 0 ? (
                <EmptyRow cols={2} />
              ) : (
                receiverIds.map((id) => (
                  <tr key={id}>
                    <td>{id}</td>
                    <VolCell value={receivers[id].events_in || 0} max={maxReceiverIn} />
                  </tr>
                ))
              )}
            </tbody>
          </table>
        </div>
        <div className="panel">
          <h2>Exporters</h2>
          <table className="data-table">
            <thead>
              <tr>
                <th>Exporter</th>
                <th className="numeric">Events in</th>
                <th className="numeric">Batches sent</th>
                <th className="numeric">Batches failed</th>
              </tr>
            </thead>
            <tbody>
              {exporterIds.length === 0 ? (
                <EmptyRow cols={4} />
              ) : (
                exporterIds.map((id) => {
                  const e = exporters[id];
                  const problem = (e.batches_failed || 0) > 0;
                  return (
                    <tr key={id} className={problem ? "row-problem" : undefined}>
                      <td>
                        {id} <ErrorIndicator lastError={e.last_error} />
                      </td>
                      <VolCell value={e.events_in} max={maxExporterIn} />
                      <td className="numeric">{e.batches_sent}</td>
                      <td className="numeric">{e.batches_failed}</td>
                    </tr>
                  );
                })
              )}
            </tbody>
          </table>
        </div>
      </div>
    </section>
  );
}
