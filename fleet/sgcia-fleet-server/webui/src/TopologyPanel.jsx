import { computeSankeyLayout, NODE_W, SANKEY_W, pctSuffix } from "./sankey.js";

// SankeyNode/SankeyLink are copied from the per-agent app's TopologyView.jsx
// (otelcol/extensions/statuscfgextension/webui-react/src/views/TopologyView.jsx)
// -- pure presentational components, no data-fetching, so a straight copy
// into this independently-built app costs nothing and needs no adaptation.

function SankeyNode({ box, type }) {
  const h = Math.max(box.height, 1);
  const style = box.color ? { stroke: box.color } : undefined;
  return (
    <g>
      <rect x={box.x} y={box.y} width={NODE_W} height={h} rx={4} className={`sankey-node sankey-node-${type}`} style={style} />
      <foreignObject x={box.x} y={box.y} width={NODE_W} height={h}>
        <div xmlns="http://www.w3.org/1999/xhtml" className="sankey-label" title={box.id}>
          <span className="sankey-label-name">{box.id}</span>
          <span className="sankey-label-count">
            {box.flow}
            {pctSuffix(box.pct)}
          </span>
        </div>
      </foreignObject>
    </g>
  );
}

function SankeyLink({ link }) {
  const midX = (link.sx + link.tx) / 2;
  const half = link.w / 2;
  const y0top = link.sy - half,
    y0bot = link.sy + half;
  const y1top = link.ty - half,
    y1bot = link.ty + half;
  const d =
    `M${link.sx},${y0top}` +
    ` C${midX},${y0top} ${midX},${y1top} ${link.tx},${y1top}` +
    ` L${link.tx},${y1bot}` +
    ` C${midX},${y1bot} ${midX},${y0bot} ${link.sx},${y0bot} Z`;
  const style = link.color ? { fill: link.color } : undefined;
  const title = `${link.from} → ${link.to}: ${link.flow}${pctSuffix(link.pct)}`;
  return (
    <path d={d} className="sankey-link" style={style}>
      <title>{title}</title>
    </path>
  );
}

// TopologyPanel renders one agent's own Sankey diagram, fed straight from
// its already-tracked snapshot -- agent.snapshot.topology is the graph,
// agent.snapshot itself (receivers/pipelines/exporters) is the status,
// exactly the two arguments computeSankeyLayout expects. This is "compare"
// not "merge": the caller renders one of these per selected agent, stacked
// under its own heading, rather than attempting a single cross-agent graph.
export default function TopologyPanel({ agent }) {
  const snap = agent.snapshot;
  const graph = snap?.topology;
  const layout = graph && snap ? computeSankeyLayout(graph, snap) : null;

  return (
    <div className="topology-panel">
      <h3>{agent.hostname || agent.id}</h3>
      {!layout ? (
        <div className="empty-state">Nothing configured yet, or no topology reported.</div>
      ) : (
        <svg viewBox={`0 0 ${SANKEY_W} ${layout.canvasHeight}`} className="sankey-svg" preserveAspectRatio="xMinYMin meet">
          {layout.links.map((l) => (
            <SankeyLink key={`${l.from}->${l.to}`} link={l} />
          ))}
          {layout.recvBoxes.map((b) => (
            <SankeyNode key={b.id} box={b} type="receiver" />
          ))}
          {layout.pipeBoxes.map((b) => (
            <SankeyNode key={b.id} box={b} type="pipeline" />
          ))}
          {layout.expBoxes.map((b) => (
            <SankeyNode key={b.id} box={b} type="exporter" />
          ))}
        </svg>
      )}
    </div>
  );
}
