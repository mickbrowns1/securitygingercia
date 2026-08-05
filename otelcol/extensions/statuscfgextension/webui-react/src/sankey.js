// Pure layout math for the Topology view's Sankey diagram -- kept
// separate from the JSX rendering so this can be tested/reasoned about
// independently of React. Ported 1:1 from the original vanilla-JS
// implementation (webui/app.js); see that file's own comments for the
// full history of *why* each of these choices was made -- summarized
// briefly here since this is a straight port, not a redesign.

// Every node and edge in this diagram uses events_in, never events_out,
// even for pipelines -- events_out is the *sum* of what a pipeline sent
// to every exporter it feeds (real fan-out replication means each
// exporter gets the pipeline's entire output, not a fraction, so with N
// exporters events_out is roughly Nx the pipeline's actual
// single-instance throughput). events_in passes straight through
// unchanged from receiver to pipeline, and an exporter's own events_in
// is a real counter that already reflects the true cumulative total of
// everything that arrived from every pipeline feeding it -- so every
// node/edge value here is a real, exact number from /status, not an
// estimate, and an apples-to-apples one across every column too.
export function flowFor(status, type, id) {
  const table = type === "receiver" ? status.receivers : type === "pipeline" ? status.pipelines : status.exporters;
  const rec = (table || {})[id];
  return rec ? rec.events_in || 0 : 0;
}

export function sumFlow(list) {
  return list.reduce((s, n) => s + n.flow, 0);
}

export const SANKEY_W = 900;
export const NODE_W = 150;
const MIN_NODE_H = 22;
const NODE_GAP = 10;
const MIN_SANKEY_H = 160;
const MAX_SANKEY_H = 720;
const PX_PER_EVENT = 0.5;
// A column with only a handful of nodes has room to give each one a
// chunkier baseline box instead of shrinking everything to the bare
// MIN_NODE_H floor -- this ramps a column's per-node baseline height
// down from ROOMY_NODE_H as node count grows, only hitting the floor
// once a column is crowded enough to need it.
const ROOMY_NODE_H = 72;
function dynamicNodeH(maxCount) {
  return Math.max(MIN_NODE_H, Math.min(ROOMY_NODE_H, (ROOMY_NODE_H * 4) / maxCount));
}

// Every ribbon used to be the same blue regardless of which pipeline it
// belonged to, which made them unreadable the moment two pipelines'
// ribbons crossed. Each pipeline gets one base color from this palette,
// used as-is for its outbound (exporter-side) ribbons and its own node
// border, so you can trace one pipeline's flow across a crossing by eye.
// Cycles if there are more pipelines than colors.
export const SANKEY_PALETTE = ["#5aa9e6", "#4caf7d", "#e0a83e", "#c77dff", "#5ee6c0", "#e0605e", "#f08fb0", "#8fa6e6"];

// Multiple receivers feeding the *same* pipeline (e.g. syslog/udp and
// syslog/tcp both into logs/syslog) all inherit that pipeline's base
// color on their inbound ribbons, which made them indistinguishable
// from each other. Each inbound ribbon into a given pipeline gets a
// shade offset from this list (by the order it's fed in), cycling if a
// pipeline has more receivers than offsets -- the first receiver still
// gets the pure base color (offset 0), matching the node border.
const SHADE_OFFSETS = [0, -28, 28, -45, 45, -14, 14];

function shadeColor(hex, percent) {
  const num = parseInt(hex.slice(1), 16);
  const r = (num >> 16) & 0xff,
    g = (num >> 8) & 0xff,
    b = num & 0xff;
  const adjust = (c) => (percent >= 0 ? c + (255 - c) * (percent / 100) : c + c * (percent / 100));
  const toHex = (c) => Math.min(255, Math.max(0, Math.round(c))).toString(16).padStart(2, "0");
  return "#" + toHex(adjust(r)) + toHex(adjust(g)) + toHex(adjust(b));
}

// Height is fit-to-content (clamped) rather than a fixed canvas -- an
// all-zero or near-empty topology used to collapse every node to its
// minimum and leave most of a fixed canvas as dead space below them. A
// genuinely busy topology still gets capped at MAX_SANKEY_H.
function pickCanvasHeight(maxCount, globalMaxTotal, nodeH) {
  const reserved = nodeH * maxCount + NODE_GAP * Math.max(maxCount - 1, 0);
  const natural = reserved + globalMaxTotal * PX_PER_EVENT;
  return Math.min(Math.max(natural, MIN_SANKEY_H), MAX_SANKEY_H);
}

// Each column is centered independently within the shared canvas height
// -- columns with fewer/smaller nodes end up with breathing room above
// and below instead of everything pinned to the top.
function layoutColumn(list, x, pxPerEvent, canvasHeight, nodeH) {
  const contentHeight = list.reduce((s, n) => s + nodeH + n.flow * pxPerEvent, 0) + NODE_GAP * Math.max(list.length - 1, 0);
  let y = Math.max((canvasHeight - contentHeight) / 2, 0);
  return list.map((nd) => {
    const height = nodeH + nd.flow * pxPerEvent;
    const box = { id: nd.id, flow: nd.flow, pct: nd.pct, color: nd.color, x, y, height, inCursor: 0 };
    y += height + NODE_GAP;
    return box;
  });
}

export function pctSuffix(pct) {
  return typeof pct === "number" ? ` (${pct}%)` : "";
}

// Computes the full layout (node boxes + link ribbons) for the given
// /topology graph and /status snapshot. Returns null if there's nothing
// configured yet. `canvasHeight` plus the three box arrays plus `links`
// is everything the rendering component needs -- no further math there.
export function computeSankeyLayout(graph, status) {
  const nodesByType = { receiver: [], exporter: [], pipeline: [] };
  (graph.nodes || []).forEach((n) => nodesByType[n.type] && nodesByType[n.type].push(n));

  if (!nodesByType.receiver.length && !nodesByType.pipeline.length && !nodesByType.exporter.length) {
    return null;
  }

  const pipelineIds = {};
  nodesByType.pipeline.forEach((p) => (pipelineIds[p.id] = true));

  const recv = nodesByType.receiver
    .map((n) => ({ id: n.id, flow: flowFor(status, "receiver", n.id) }))
    .sort((a, b) => b.flow - a.flow || a.id.localeCompare(b.id));
  const pipe = nodesByType.pipeline
    .map((n) => ({ id: n.id, flow: flowFor(status, "pipeline", n.id) }))
    .sort((a, b) => b.flow - a.flow || a.id.localeCompare(b.id));
  pipe.forEach((p, i) => (p.color = SANKEY_PALETTE[i % SANKEY_PALETTE.length]));
  const exp = nodesByType.exporter
    .map((n) => ({ id: n.id, flow: flowFor(status, "exporter", n.id) }))
    .sort((a, b) => b.flow - a.flow || a.id.localeCompare(b.id));

  // Total ingested logs (every receiver's events_in, summed) -- the one
  // number in this diagram that's never inflated by a pipeline
  // replicating its output to more than one exporter, so it's a stable
  // "100%" reference for every node and ribbon below.
  const totalLogs = sumFlow(recv);
  const pctOf = (flow) => (totalLogs > 0 ? Math.round((flow / totalLogs) * 100) : 0);
  recv.forEach((n) => (n.pct = pctOf(n.flow)));
  pipe.forEach((n) => (n.pct = pctOf(n.flow)));
  exp.forEach((n) => (n.pct = pctOf(n.flow)));

  // One shared scale across the whole diagram (not per-column) so a
  // link's width matches at both ends instead of visibly tapering.
  const maxCount = Math.max(recv.length, pipe.length, exp.length, 1);
  const globalMaxTotal = Math.max(sumFlow(recv), sumFlow(pipe), sumFlow(exp));
  const nodeH = dynamicNodeH(maxCount);
  const canvasHeight = pickCanvasHeight(maxCount, globalMaxTotal, nodeH);
  const reserved = nodeH * maxCount + NODE_GAP * (maxCount - 1);
  const flexBudget = Math.max(canvasHeight - reserved, 0);
  const pxPerEvent = globalMaxTotal > 0 ? flexBudget / globalMaxTotal : 0;

  const colGap = (SANKEY_W - NODE_W * 3) / 2;
  const recvBoxes = layoutColumn(recv, 0, pxPerEvent, canvasHeight, nodeH);
  const pipeBoxes = layoutColumn(pipe, NODE_W + colGap, pxPerEvent, canvasHeight, nodeH);
  const expBoxes = layoutColumn(exp, (NODE_W + colGap) * 2, pxPerEvent, canvasHeight, nodeH);

  const byId = {};
  recvBoxes.forEach((b) => (byId[b.id] = b));
  pipeBoxes.forEach((b) => (byId[b.id] = b));
  expBoxes.forEach((b) => (byId[b.id] = b));

  // Color and width are known per edge without touching layout, so
  // compute those first in one pass. inboundSeen tracks, per pipeline,
  // how many of its receiver-side edges have been assigned a shade so
  // far, so each one gets a different offset from SHADE_OFFSETS.
  const inboundSeen = {};
  const rawLinks = (graph.edges || [])
    .map((e) => {
      const s = byId[e.from],
        t = byId[e.to];
      if (!s || !t) return null;
      const flow = pipelineIds[e.to] ? flowFor(status, "receiver", e.from) : flowFor(status, "pipeline", e.from);
      let color;
      if (pipelineIds[e.to]) {
        // Receiver -> pipeline: shade by which receiver this is, so two
        // receivers feeding the same pipeline don't render identically.
        const seen = inboundSeen[e.to] || 0;
        inboundSeen[e.to] = seen + 1;
        color = shadeColor(t.color, SHADE_OFFSETS[seen % SHADE_OFFSETS.length]);
      } else {
        // Pipeline -> exporter: keep the pure base color unshaded, so
        // the pipeline stays traceable by one consistent hue across
        // every exporter it feeds, even as ribbons cross.
        color = s.color;
      }
      // Even a zero-flow edge gets a real, visible ribbon (2px) rather
      // than fading into an unreadable hairline -- the point is to show
      // that a connection exists structurally, whether or not it's
      // carrying traffic right now.
      const w = Math.max(flow * pxPerEvent, 2);
      return { s, t, w, color, from: e.from, to: e.to, flow, pct: pctOf(flow) };
    })
    .filter(Boolean);

  // Incoming edges into a node are a real merge of distinct upstream
  // sources (e.g. two receivers feeding one pipeline) -- genuinely
  // additive, so they stack and center as a group within the node's
  // height. Outgoing edges are different: every pipeline/exporter
  // attached to a node receives the *same* complete stream, not a split
  // of it, so they all attach at the same point -- the node's own
  // vertical center -- instead of competing for stacked sub-bands.
  const totalIn = {};
  rawLinks.forEach((l) => (totalIn[l.t.id] = (totalIn[l.t.id] || 0) + l.w));
  Object.keys(totalIn).forEach((id) => {
    const box = byId[id];
    box.inCursor = Math.max((box.height - totalIn[id]) / 2, 0);
  });

  const links = rawLinks.map((l) => {
    const { s, t, w } = l;
    const sy = s.y + s.height / 2;
    const ty = t.y + t.inCursor + w / 2;
    t.inCursor += w;
    return { sx: s.x + NODE_W, sy, tx: t.x, ty, w, color: l.color, from: l.from, to: l.to, flow: l.flow, pct: l.pct };
  });

  return { canvasHeight, recvBoxes, pipeBoxes, expBoxes, links };
}
