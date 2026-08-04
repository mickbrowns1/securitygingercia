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

  // volBarStyle renders relative volume (this row's count against the
  // busiest row in the same table) as a subtle background gradient --
  // a cheap, layout-free stand-in for a real bar chart, using data
  // that's already being polled for the numbers themselves.
  function volBarStyle(value, max) {
    var pct = max > 0 ? Math.round((value / max) * 100) : 0;
    return ' style="background: linear-gradient(to right, var(--vol-bar) ' + pct + '%, transparent ' + pct + '%)"';
  }

  function renderPipelines(pipelines) {
    var names = Object.keys(pipelines || {}).sort();
    var max = 0;
    names.forEach(function (n) { max = Math.max(max, pipelines[n].events_in || 0); });
    var rows = names.map(function (name) {
      var p = pipelines[name];
      return "<tr" + volBarStyle(p.events_in, max) + "><td>" + escapeHTML(name) + "</td><td class=\"numeric\">" + p.events_in +
        "</td><td class=\"numeric\">" + p.events_out + "</td><td class=\"numeric\">" + p.events_dropped + "</td></tr>";
    });
    $("pipelines-body").innerHTML = rows.join("") || emptyRow(4);
  }

  function renderReceivers(receivers) {
    var ids = Object.keys(receivers || {}).sort();
    var max = 0;
    ids.forEach(function (id) { max = Math.max(max, receivers[id].events_in || 0); });
    var rows = ids.map(function (id) {
      var v = receivers[id].events_in || 0;
      return "<tr" + volBarStyle(v, max) + "><td>" + escapeHTML(id) + "</td><td class=\"numeric\">" + v + "</td></tr>";
    });
    $("receivers-body").innerHTML = rows.join("") || emptyRow(2);
  }

  function renderExporters(exporters) {
    var ids = Object.keys(exporters || {}).sort();
    var max = 0;
    ids.forEach(function (id) { max = Math.max(max, exporters[id].events_in || 0); });
    var rows = ids.map(function (id) {
      var e = exporters[id];
      return "<tr" + volBarStyle(e.events_in, max) + "><td>" + escapeHTML(id) + "</td><td class=\"numeric\">" + e.events_in +
        "</td><td class=\"numeric\">" + e.batches_sent + "</td><td class=\"numeric\">" + e.batches_failed + "</td></tr>";
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
      updateSeverityOptions(entries);
      renderLogs(entries);
    }).catch(function (err) { setConnStatus(false, err.message); });
  }

  $("logs-refresh").addEventListener("click", loadLogs);
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

  // ---- Topology view ----

  function renderTopology(graph) {
    var nodesByType = { receiver: [], exporter: [], pipeline: [] };
    (graph.nodes || []).forEach(function (n) { nodesByType[n.type] && nodesByType[n.type].push(n); });

    var inbound = {}, outbound = {};
    (graph.edges || []).forEach(function (edge) {
      var pipelineIsTarget = nodesByType.pipeline.some(function (p) { return p.id === edge.to; });
      if (pipelineIsTarget) {
        (inbound[edge.to] = inbound[edge.to] || []).push(edge.from);
      } else {
        (outbound[edge.from] = outbound[edge.from] || []).push(edge.to);
      }
    });

    var receiversHTML = nodesByType.receiver.map(function (n) {
      return '<div class="topo-node">' + escapeHTML(n.id) + "</div>";
    }).join("") || '<div class="topo-node" style="color:var(--muted)">none configured</div>';

    var exportersHTML = nodesByType.exporter.map(function (n) {
      return '<div class="topo-node">' + escapeHTML(n.id) + "</div>";
    }).join("") || '<div class="topo-node" style="color:var(--muted)">none configured</div>';

    var pipelinesHTML = nodesByType.pipeline.map(function (n) {
      var ins = (inbound[n.id] || []).map(function (id) {
        return '<span class="topo-badge in">' + escapeHTML(id) + "</span>";
      }).join("");
      var outs = (outbound[n.id] || []).map(function (id) {
        return '<span class="topo-badge out">' + escapeHTML(id) + "</span>";
      }).join("");
      return '<div class="topo-node topo-pipeline"><div class="pipe-name">' + escapeHTML(n.id) +
        '</div><div class="topo-badges">' + ins + outs + "</div></div>";
    }).join("") || '<div class="topo-node" style="color:var(--muted)">none configured</div>';

    $("topology-graph").innerHTML =
      '<div class="topology-col"><h3>Receivers</h3>' + receiversHTML + "</div>" +
      '<div class="topology-col"><h3>Pipelines</h3>' + pipelinesHTML + "</div>" +
      '<div class="topology-col"><h3>Exporters</h3>' + exportersHTML + "</div>";
  }

  function loadTopology() {
    fetchJSON("topology").then(function (graph) {
      setConnStatus(true);
      renderTopology(graph);
    }).catch(function (err) { setConnStatus(false, err.message); });
  }

  // ---- Polling ----

  function startPolling() {
    clearInterval(state.healthTimer);
    clearInterval(state.logsTimer);
    state.healthTimer = setInterval(function () {
      if (state.activeView === "health") loadHealth();
    }, HEALTH_POLL_MS);
    state.logsTimer = setInterval(function () {
      if (state.activeView === "logs" && $("logs-autorefresh").checked) loadLogs();
    }, LOGS_POLL_MS);
  }

  loadHealth();
  startPolling();
})();
