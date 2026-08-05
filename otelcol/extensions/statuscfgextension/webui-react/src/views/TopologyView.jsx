import { useState } from "react";
import { fetchJSON } from "../api.js";
import { useInterval } from "../useInterval.js";
import { computeSankeyLayout, NODE_W, SANKEY_W, pctSuffix } from "../sankey.js";

const TOPOLOGY_POLL_MS = 5000;

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

export default function TopologyView({ active, onConnStatus }) {
  const [graph, setGraph] = useState(null);
  const [status, setStatus] = useState(null);

  useInterval(
    () => {
      Promise.all([fetchJSON("topology"), fetchJSON("status")])
        .then(([g, s]) => {
          onConnStatus(true);
          setGraph(g);
          setStatus(s);
        })
        .catch((err) => onConnStatus(false, err.message));
    },
    TOPOLOGY_POLL_MS,
    active
  );

  const layout = graph && status ? computeSankeyLayout(graph, status) : null;

  return (
    <section className={"view" + (active ? " active" : "")} id="view-topology">
      <div className="panel">
        <h2>Pipeline topology</h2>
        <div className="sankey-headers">
          <span>Receivers</span>
          <span>Pipelines</span>
          <span>Exporters</span>
        </div>
        <div id="topology-graph">
          {!layout ? (
            <div className="topo-empty">Nothing configured yet.</div>
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
      </div>
    </section>
  );
}
