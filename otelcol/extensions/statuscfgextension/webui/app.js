(function () {
  "use strict";

  var HEALTH_POLL_MS = 5000;
  var LOGS_POLL_MS = 4000;

  var state = {
    activeView: "health",
    knownSeverities: new Set(),
    logsTimer: null,
    healthTimer: null,
    corrFilter: null, // { key, value } | null -- set by clicking an attribute badge in the Logs view
    lastLogEntries: [], // whatever /logs last returned -- Export downloads exactly this, filters and all
  };

  function $(id) { return document.getElementById(id); }

  function setConnStatus(ok, detail) {
    var el = $("conn-status");
    el.className = ok ? "ok" : "err";
    el.textContent = ok ? "connected" : "connection error: " + detail;
  }

  function fetchJSON(path) {
    return fetch(path, { cache: "no-store" }).then(function (resp) {
      if (!resp.ok) throw new Error(resp.status + " " + resp.statusText);
      return resp.json();
    });
  }

  function escapeHTML(s) {
    return String(s).replace(/[&<>"']/g, function (c) {
      return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c];
    });
  }

  // ---- Tabs ----

  function switchView(view) {
    state.activeView = view;
    document.querySelectorAll(".tab-btn").forEach(function (btn) {
      btn.classList.toggle("active", btn.dataset.view === view);
    });
    document.querySelectorAll(".view").forEach(function (section) {
      section.classList.toggle("active", section.id === "view-" + view);
    });
    if (view === "health") loadHealth();
    if (view === "logs") loadLogs();
    if (view === "topology") loadTopology();
  }

  document.querySelectorAll(".tab-btn").forEach(function (btn) {
    btn.addEventListener("click", function () { switchView(btn.dataset.view); });
  });

  // ---- Health view ----

  function renderSummary(snapshot) {
    var uptime = formatUptime(snapshot.uptime_seconds);
    var receiverCount = Object.keys(snapshot.receivers || {}).length;
    var pipelineCount = Object.keys(snapshot.pipelines || {}).length;
    var exporterCount = Object.keys(snapshot.exporters || {}).length;
    var items = [
      ["Uptime", uptime],
      ["Receivers", receiverCount],
      ["Pipelines", pipelineCount],
      ["Exporters", exporterCount],
    ];
    $("health-summary").innerHTML = items.map(function (pair) {
      return '<div class="summary-item"><div class="label">' + escapeHTML(pair[0]) +
        '</div><div class="value">' + escapeHTML(pair[1]) + "</div></div>";
    }).join("");
  }

  function formatUptime(seconds) {
    seconds = seconds || 0;
    var h = Math.floor(seconds / 3600);
    var m = Math.floor((seconds % 3600) / 60);
    var s = seconds % 60;
    if (h > 0) return h + "h " + m + "m";
    if (m > 0) return m + "m " + s + "s";
    return s + "s";
  }

  // volCell renders an explicit visible bar (not just a background
  // tint -- that was tried first and was too subtle to notice in
  // practice) showing this row's count relative to the busiest row in
  // the same table, with the number right above it.
  function volCell(value, max) {
    var pct = max > 0 ? Math.round((value / max) * 100) : 0;
    return '<td class="numeric vol-cell"><span class="vol-number">' + value + '</span>' +
      '<span class="vol-track"><span class="vol-fill" style="width:' + pct + '%"></span></span></td>';
  }

  function renderPipelines(pipelines) {
    var names = Object.keys(pipelines || {}).sort();
    var max = 0;
    names.forEach(function (n) { max = Math.max(max, pipelines[n].events_in || 0); });
    var rows = names.map(function (name) {
      var p = pipelines[name];
      var problem = (p.events_dropped || 0) > 0 || (p.parse_errors || 0) > 0;
      return "<tr" + (problem ? ' class="row-problem"' : "") + "><td>" + escapeHTML(name) + "</td>" + volCell(p.events_in, max) +
        "<td class=\"numeric\">" + p.events_out + "</td><td class=\"numeric\">" + p.events_dropped + "</td></tr>";
    });
    $("pipelines-body").innerHTML = rows.join("") || emptyRow(4);
  }

  function renderReceivers(receivers) {
    var ids = Object.keys(receivers || {}).sort();
    var max = 0;
    ids.forEach(function (id) { max = Math.max(max, receivers[id].events_in || 0); });
    var rows = ids.map(function (id) {
      var v = receivers[id].events_in || 0;
      return "<tr><td>" + escapeHTML(id) + "</td>" + volCell(v, max) + "</tr>";
    });
    $("receivers-body").innerHTML = rows.join("") || emptyRow(2);
  }

  // last_error (message + timestamp) already comes back from /status but
  // previously had nowhere to show up in the UI at all -- the only way to
  // see *why* an exporter was failing was journalctl on the box itself.
  function errorIndicator(lastError) {
    if (!lastError || !lastError.message) return "";
    var when = lastError.at ? new Date(lastError.at).toLocaleString() : "unknown time";
    return ' <span class="err-indicator" title="' + escapeHTML(when + ": " + lastError.message) + '">!</span>';
  }

  function renderExporters(exporters) {
    var ids = Object.keys(exporters || {}).sort();
    var max = 0;
    ids.forEach(function (id) { max = Math.max(max, exporters[id].events_in || 0); });
    var rows = ids.map(function (id) {
      var e = exporters[id];
      var problem = (e.batches_failed || 0) > 0;
      return "<tr" + (problem ? ' class="row-problem"' : "") + "><td>" + escapeHTML(id) + errorIndicator(e.last_error) + "</td>" + volCell(e.events_in, max) +
        "<td class=\"numeric\">" + e.batches_sent + "</td><td class=\"numeric\">" + e.batches_failed + "</td></tr>";
    });
    $("exporters-body").innerHTML = rows.join("") || emptyRow(4);
  }

  function emptyRow(cols) {
    return '<tr><td colspan="' + cols + '" style="color:var(--muted)">no data yet</td></tr>';
  }

  function loadHealth() {
    fetchJSON("status").then(function (snapshot) {
      setConnStatus(true);
      renderSummary(snapshot);
      renderPipelines(snapshot.pipelines);
      renderReceivers(snapshot.receivers);
      renderExporters(snapshot.exporters);
    }).catch(function (err) { setConnStatus(false, err.message); });
  }

  // ---- Logs view ----

  function severityClass(sev) {
    var s = (sev || "").toLowerCase();
    if (s.indexOf("err") !== -1 || s.indexOf("fatal") !== -1) return "sev-error";
    if (s.indexOf("warn") !== -1) return "sev-warn";
    if (s.indexOf("info") !== -1) return "sev-info";
    if (s.indexOf("debug") !== -1) return "sev-debug";
    if (s.indexOf("trace") !== -1) return "sev-trace";
    return "sev-info";
  }

  function updateSeverityOptions(entries) {
    var select = $("logs-severity");
    var current = select.value;
    var changed = false;
    entries.forEach(function (e) {
      if (e.severity && !state.knownSeverities.has(e.severity)) {
        state.knownSeverities.add(e.severity);
        changed = true;
      }
    });
    if (!changed) return;
    var sevs = Array.from(state.knownSeverities).sort();
    select.innerHTML = '<option value="">All severities</option>' + sevs.map(function (s) {
      return '<option value="' + escapeHTML(s) + '">' + escapeHTML(s) + "</option>";
    }).join("");
    select.value = current;
  }

  // Each attribute/resource pair renders as a clickable badge -- click
  // one to filter the log view to only other events with that exact
  // key=value (a lightweight correlation tool, e.g. "what else happened
  // on this host/session?"). Exact match, not substring -- see the
  // matching backend note in logbuffer.go's Snapshot for why that
  // matters.
  function renderAttrBadges(entry) {
    var parts = [];
    function addBadges(map) {
      if (!map) return;
      Object.keys(map).forEach(function (k) {
        var v = map[k];
        parts.push('<span class="corr-badge" data-key="' + escapeHTML(k) + '" data-value="' + escapeHTML(v) +
          '" title="Click to filter to events with ' + escapeHTML(k) + "=" + escapeHTML(v) + '">' +
          escapeHTML(k) + "=" + escapeHTML(v) + "</span>");
      });
    }
    addBadges(entry.attributes);
    addBadges(entry.resource);
    return parts.join("");
  }

  function updateCorrChip() {
    var chip = $("corr-chip");
    if (!state.corrFilter) {
      chip.style.display = "none";
      chip.innerHTML = "";
      return;
    }
    chip.style.display = "flex";
    chip.innerHTML = "Filtering: <strong>" + escapeHTML(state.corrFilter.key) + "=" + escapeHTML(state.corrFilter.value) +
      '</strong> <button id="corr-clear" type="button">&times;</button>';
    $("corr-clear").addEventListener("click", function () {
      state.corrFilter = null;
      updateCorrChip();
      loadLogs();
    });
  }

  function renderLogs(entries) {
    var filterActive = !!(state.corrFilter || $("logs-query").value.trim() || $("logs-severity").value);
    var emptyEl = $("logs-empty");
    if (entries.length === 0) {
      emptyEl.style.display = "block";
      emptyEl.innerHTML = filterActive
        ? "No matching events."
        : 'No log records yet. Add a <code>logbuffer</code> exporter to a logs pipeline to see events here.';
    } else {
      emptyEl.style.display = "none";
    }
    var rows = entries.slice().reverse().map(function (e) {
      var t = e.timestamp ? new Date(e.timestamp).toLocaleTimeString() : "";
      return "<tr><td>" + escapeHTML(t) + '</td><td><span class="sev-badge ' + severityClass(e.severity) +
        '">' + escapeHTML(e.severity || "") + '</span></td><td class="body-cell">' + escapeHTML(e.body) +
        '</td><td class="attrs-cell">' + renderAttrBadges(e) + "</td></tr>";
    });
    $("logs-body").innerHTML = rows.join("");
  }

  function loadLogs() {
    var severity = $("logs-severity").value;
    var params = new URLSearchParams();
    if (state.corrFilter) {
      params.set("attr_key", state.corrFilter.key);
      params.set("attr_value", state.corrFilter.value);
    } else {
      var q = $("logs-query").value.trim();
      if (q) params.set("q", q);
    }
    if (severity) params.set("severity", severity);
    var path = "logs" + (params.toString() ? "?" + params.toString() : "");
    fetchJSON(path).then(function (entries) {
      setConnStatus(true);
      state.lastLogEntries = entries;
      updateSeverityOptions(entries);
      renderLogs(entries);
    }).catch(function (err) { setConnStatus(false, err.message); });
  }

  // Downloads exactly what's currently shown (whatever query/severity/
  // correlation filter is active) as a JSON file -- e.g. to attach to a
  // ticket or hand to a colleague without them needing access to this
  // collector's web UI at all.
  function exportLogs() {
    var data = JSON.stringify(state.lastLogEntries, null, 2);
    var blob = new Blob([data], { type: "application/json" });
    var url = URL.createObjectURL(blob);
    var a = document.createElement("a");
    a.href = url;
    a.download = "sgcia-logs-" + new Date().toISOString().replace(/[:.]/g, "-") + ".json";
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
    fetch("logs", { method: "DELETE" }).then(function (resp) {
      if (!resp.ok) throw new Error(resp.status + " " + resp.statusText);
      setConnStatus(true);
      loadLogs();
    }).catch(function (err) { setConnStatus(false, err.message); });
  }

  // Cycles through currently-rendered ERROR/FATAL rows (n = forward,
  // p = backward), highlighting and scrolling to each in turn. Recomputed
  // from the live DOM on every call rather than cached, since the table
  // can be replaced by the next auto-refresh at any moment -- losing the
  // current position across a refresh (falling back to "start from the
  // top") is an acceptable tradeoff for not tracking stale rows.
  function jumpToError(direction) {
    var rows = Array.prototype.slice.call(document.querySelectorAll("#logs-body tr"));
    var errorRows = rows.filter(function (r) { return r.querySelector(".sev-error, .sev-fatal"); });
    if (errorRows.length === 0) return;
    var currentIndex = errorRows.findIndex(function (r) { return r.classList.contains("nav-highlight"); });
    errorRows.forEach(function (r) { r.classList.remove("nav-highlight"); });
    var nextIndex = currentIndex === -1
      ? (direction > 0 ? 0 : errorRows.length - 1)
      : (currentIndex + direction + errorRows.length) % errorRows.length;
    var target = errorRows[nextIndex];
    target.classList.add("nav-highlight");
    target.scrollIntoView({ block: "nearest" });
  }

  $("logs-refresh").addEventListener("click", loadLogs);
  $("logs-export").addEventListener("click", exportLogs);
  $("logs-clear").addEventListener("click", clearLogBuffer);
  $("logs-query").addEventListener("keydown", function (ev) {
    if (ev.key !== "Enter") return;
    state.corrFilter = null;
    updateCorrChip();
    loadLogs();
  });
  $("logs-severity").addEventListener("change", loadLogs);
  $("logs-body").addEventListener("click", function (ev) {
    var badge = ev.target.closest(".corr-badge");
    if (!badge) return;
    state.corrFilter = { key: badge.dataset.key, value: badge.dataset.value };
    $("logs-query").value = "";
    updateCorrChip();
    loadLogs();
  });

  // ---- Topology view (Sankey diagram) ----

  // A receiver's own events_in exactly sizes its edge into whichever
  // pipeline it feeds (a receiver belongs to at most one pipeline in
  // practice), and a pipeline's events_out exactly sizes its edge into
  // *every* exporter it feeds -- each exporter attached to a pipeline
  // gets the pipeline's entire output, not a fraction of it. So unlike
  // a purely decorative diagram, every link width below is a real,
  // exact number from /status, not an estimate.
  function flowFor(status, type, id) {
    var table = type === "receiver" ? status.receivers : type === "pipeline" ? status.pipelines : status.exporters;
    var rec = (table || {})[id];
    if (!rec) return 0;
    return type === "pipeline" ? (rec.events_out || 0) : (rec.events_in || 0);
  }

  function sumFlow(list) {
    return list.reduce(function (s, n) { return s + n.flow; }, 0);
  }

  var SANKEY_W = 900, NODE_W = 150, MIN_NODE_H = 22, NODE_GAP = 10;
  var MIN_SANKEY_H = 140, MAX_SANKEY_H = 420, PX_PER_EVENT = 0.5;

  // Every ribbon used to be the same blue regardless of which pipeline
  // it belonged to, which made them unreadable the moment two pipelines'
  // ribbons crossed (exactly what happens once they fan out to shared
  // exporters, which every example config here does). Each pipeline
  // gets one color from this palette, used consistently for both its
  // inbound (receiver-side) and outbound (exporter-side) ribbons and its
  // own node border, so you can trace one pipeline's flow across a
  // crossing by eye. Cycles if there are more pipelines than colors.
  var SANKEY_PALETTE = ["#5aa9e6", "#4caf7d", "#e0a83e", "#c77dff", "#5ee6c0", "#e0605e", "#f08fb0", "#8fa6e6"];

  // Height is fit-to-content (clamped) rather than a fixed canvas --
  // an all-zero or near-empty topology used to collapse every node to
  // its minimum and leave most of a fixed 420px canvas as dead space
  // below them, which is what actually prompted this. A genuinely busy
  // topology still gets capped at MAX_SANKEY_H, same as before.
  function pickCanvasHeight(maxCount, globalMaxTotal) {
    var reserved = MIN_NODE_H * maxCount + NODE_GAP * Math.max(maxCount - 1, 0);
    var natural = reserved + globalMaxTotal * PX_PER_EVENT;
    return Math.min(Math.max(natural, MIN_SANKEY_H), MAX_SANKEY_H);
  }

  // Each column is centered independently within the shared canvas
  // height -- columns with fewer/smaller nodes end up with breathing
  // room above and below instead of everything pinned to the top.
  function layoutColumn(list, x, pxPerEvent, canvasHeight) {
    var contentHeight = list.reduce(function (s, n) { return s + MIN_NODE_H + n.flow * pxPerEvent; }, 0) +
      NODE_GAP * Math.max(list.length - 1, 0);
    var y = Math.max((canvasHeight - contentHeight) / 2, 0);
    return list.map(function (nd) {
      var height = MIN_NODE_H + nd.flow * pxPerEvent;
      var box = { id: nd.id, flow: nd.flow, color: nd.color, x: x, y: y, height: height, inCursor: 0, outCursor: 0 };
      y += height + NODE_GAP;
      return box;
    });
  }

  function sankeyNodeSVG(box, type) {
    var h = Math.max(box.height, 1);
    var strokeStyle = box.color ? ' style="stroke:' + box.color + '"' : "";
    return '<g>' +
      '<rect x="' + box.x + '" y="' + box.y + '" width="' + NODE_W + '" height="' + h +
      '" rx="4" class="sankey-node sankey-node-' + type + '"' + strokeStyle + '></rect>' +
      '<foreignObject x="' + box.x + '" y="' + box.y + '" width="' + NODE_W + '" height="' + h + '">' +
      '<div xmlns="http://www.w3.org/1999/xhtml" class="sankey-label" title="' + escapeHTML(box.id) + '">' +
      '<span class="sankey-label-name">' + escapeHTML(box.id) + '</span>' +
      '<span class="sankey-label-count">' + box.flow + '</span>' +
      '</div></foreignObject></g>';
  }

  function sankeyLinkSVG(link) {
    var midX = (link.sx + link.tx) / 2;
    var half = link.w / 2;
    var y0top = link.sy - half, y0bot = link.sy + half;
    var y1top = link.ty - half, y1bot = link.ty + half;
    var d = "M" + link.sx + "," + y0top +
      " C" + midX + "," + y0top + " " + midX + "," + y1top + " " + link.tx + "," + y1top +
      " L" + link.tx + "," + y1bot +
      " C" + midX + "," + y1bot + " " + midX + "," + y0bot + " " + link.sx + "," + y0bot + " Z";
    var fillStyle = link.color ? ' style="fill:' + link.color + '"' : "";
    return '<path d="' + d + '" class="sankey-link"' + fillStyle + '><title>' + escapeHTML(link.from + " → " + link.to + ": " + link.flow) + "</title></path>";
  }

  function renderTopology(graph, status) {
    var nodesByType = { receiver: [], exporter: [], pipeline: [] };
    (graph.nodes || []).forEach(function (n) { nodesByType[n.type] && nodesByType[n.type].push(n); });

    if (!nodesByType.receiver.length && !nodesByType.pipeline.length && !nodesByType.exporter.length) {
      $("topology-graph").innerHTML = '<div class="topo-empty">Nothing configured yet.</div>';
      return;
    }

    var pipelineIds = {};
    nodesByType.pipeline.forEach(function (p) { pipelineIds[p.id] = true; });

    var recv = nodesByType.receiver.map(function (n) { return { id: n.id, flow: flowFor(status, "receiver", n.id) }; })
      .sort(function (a, b) { return b.flow - a.flow || a.id.localeCompare(b.id); });
    var pipe = nodesByType.pipeline.map(function (n) { return { id: n.id, flow: flowFor(status, "pipeline", n.id) }; })
      .sort(function (a, b) { return b.flow - a.flow || a.id.localeCompare(b.id); });
    pipe.forEach(function (p, i) { p.color = SANKEY_PALETTE[i % SANKEY_PALETTE.length]; });
    var exp = nodesByType.exporter.map(function (n) { return { id: n.id, flow: flowFor(status, "exporter", n.id) }; })
      .sort(function (a, b) { return b.flow - a.flow || a.id.localeCompare(b.id); });

    // One shared scale across the whole diagram (not per-column) so a
    // link's width matches at both ends instead of visibly tapering.
    var maxCount = Math.max(recv.length, pipe.length, exp.length, 1);
    var globalMaxTotal = Math.max(sumFlow(recv), sumFlow(pipe), sumFlow(exp));
    var canvasHeight = pickCanvasHeight(maxCount, globalMaxTotal);
    var reserved = MIN_NODE_H * maxCount + NODE_GAP * (maxCount - 1);
    var flexBudget = Math.max(canvasHeight - reserved, 0);
    var pxPerEvent = globalMaxTotal > 0 ? flexBudget / globalMaxTotal : 0;

    var colGap = (SANKEY_W - NODE_W * 3) / 2;
    var recvBoxes = layoutColumn(recv, 0, pxPerEvent, canvasHeight);
    var pipeBoxes = layoutColumn(pipe, NODE_W + colGap, pxPerEvent, canvasHeight);
    var expBoxes = layoutColumn(exp, (NODE_W + colGap) * 2, pxPerEvent, canvasHeight);

    var byId = {};
    recvBoxes.forEach(function (b) { byId[b.id] = b; });
    pipeBoxes.forEach(function (b) { byId[b.id] = b; });
    expBoxes.forEach(function (b) { byId[b.id] = b; });

    var links = (graph.edges || []).map(function (e) {
      var s = byId[e.from], t = byId[e.to];
      if (!s || !t) return null;
      var flow = pipelineIds[e.to] ? flowFor(status, "receiver", e.from) : flowFor(status, "pipeline", e.from);
      // Color every ribbon by whichever endpoint is the pipeline, so a
      // pipeline's two edges (receiver-in and exporter-out) share a color
      // and stay traceable through crossings once exporters fan out.
      var color = pipelineIds[e.to] ? t.color : s.color;
      // Even a zero-flow edge gets a real, visible ribbon (2px) rather
      // than fading into an unreadable hairline -- the point is to show
      // that a connection exists structurally, whether or not it's
      // carrying traffic right now.
      var w = Math.max(flow * pxPerEvent, 2);
      var sy = s.y + s.outCursor + w / 2;
      s.outCursor += w;
      var ty = t.y + t.inCursor + w / 2;
      t.inCursor += w;
      return { sx: s.x + NODE_W, sy: sy, tx: t.x, ty: ty, w: w, color: color, from: e.from, to: e.to, flow: flow };
    }).filter(Boolean);

    var svg = '<svg viewBox="0 0 ' + SANKEY_W + ' ' + canvasHeight + '" class="sankey-svg" preserveAspectRatio="xMinYMin meet">' +
      links.map(sankeyLinkSVG).join("") +
      recvBoxes.map(function (b) { return sankeyNodeSVG(b, "receiver"); }).join("") +
      pipeBoxes.map(function (b) { return sankeyNodeSVG(b, "pipeline"); }).join("") +
      expBoxes.map(function (b) { return sankeyNodeSVG(b, "exporter"); }).join("") +
      "</svg>";
    $("topology-graph").innerHTML = svg;
  }

  function loadTopology() {
    Promise.all([fetchJSON("topology"), fetchJSON("status")]).then(function (results) {
      setConnStatus(true);
      renderTopology(results[0], results[1]);
    }).catch(function (err) { setConnStatus(false, err.message); });
  }

  // ---- Keyboard shortcuts ----

  function isTypingTarget(el) {
    return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT");
  }

  function helpVisible() { return $("help-overlay").style.display !== "none"; }
  function showHelp() { $("help-overlay").style.display = "flex"; }
  function hideHelp() { $("help-overlay").style.display = "none"; }
  function toggleHelp() { helpVisible() ? hideHelp() : showHelp(); }

  $("help-overlay").addEventListener("click", function (ev) {
    if (ev.target.id === "help-overlay") hideHelp();
  });

  document.addEventListener("keydown", function (ev) {
    var typing = isTypingTarget(document.activeElement);

    if (ev.key === "?" && !typing) {
      ev.preventDefault();
      toggleHelp();
      return;
    }

    if (ev.key === "Escape") {
      if (helpVisible()) { hideHelp(); return; }
      if (typing) document.activeElement.blur();
      if (state.corrFilter || $("logs-query").value) {
        state.corrFilter = null;
        $("logs-query").value = "";
        updateCorrChip();
        if (state.activeView === "logs") loadLogs();
      }
      return;
    }

    if (helpVisible() || typing) return;

    if (ev.key === "/") {
      ev.preventDefault();
      $("logs-query").focus();
    } else if (ev.key === "h" || ev.key === "l" || ev.key === "t") {
      switchView({ h: "health", l: "logs", t: "topology" }[ev.key]);
    } else if ((ev.key === "n" || ev.key === "p") && state.activeView === "logs") {
      jumpToError(ev.key === "n" ? 1 : -1);
    }
  });

  // ---- Polling ----

  function startPolling() {
    clearInterval(state.healthTimer);
    clearInterval(state.logsTimer);
    state.healthTimer = setInterval(function () {
      if (state.activeView === "health") loadHealth();
      if (state.activeView === "topology") loadTopology();
    }, HEALTH_POLL_MS);
    state.logsTimer = setInterval(function () {
      if (state.activeView === "logs" && $("logs-autorefresh").checked) loadLogs();
    }, LOGS_POLL_MS);
  }

  loadHealth();
  startPolling();
})();
