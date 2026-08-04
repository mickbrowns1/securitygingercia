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
      return "<tr><td>" + escapeHTML(name) + "</td>" + volCell(p.events_in, max) +
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

  function renderExporters(exporters) {
    var ids = Object.keys(exporters || {}).sort();
    var max = 0;
    ids.forEach(function (id) { max = Math.max(max, exporters[id].events_in || 0); });
    var rows = ids.map(function (id) {
      var e = exporters[id];
      return "<tr><td>" + escapeHTML(id) + "</td>" + volCell(e.events_in, max) +
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
    }, HEALTH_POLL_MS);
    state.logsTimer = setInterval(function () {
      if (state.activeView === "logs" && $("logs-autorefresh").checked) loadLogs();
    }, LOGS_POLL_MS);
  }

  loadHealth();
  startPolling();
})();
