import { useCallback, useEffect, useRef, useState } from "react";
import HealthView from "./views/HealthView.jsx";
import LogsView from "./views/LogsView.jsx";
import TopologyView from "./views/TopologyView.jsx";
import HelpOverlay from "./HelpOverlay.jsx";

const TABS = [
  { id: "health", label: "Health" },
  { id: "logs", label: "Logs" },
  { id: "topology", label: "Topology" },
];

function isTypingTarget(el) {
  return !!el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT");
}

export default function App() {
  const [activeView, setActiveView] = useState("health");
  const [connStatus, setConnStatus] = useState({ ok: null, detail: "" }); // ok===null -> "connecting..."
  const [showHelp, setShowHelp] = useState(false);
  const logsRef = useRef(null);

  const onConnStatus = useCallback((ok, detail) => setConnStatus({ ok, detail: detail || "" }), []);

  useEffect(() => {
    function onKeyDown(ev) {
      const typing = isTypingTarget(document.activeElement);

      if (ev.key === "?" && !typing) {
        ev.preventDefault();
        setShowHelp((v) => !v);
        return;
      }

      if (ev.key === "Escape") {
        if (showHelp) {
          setShowHelp(false);
          return;
        }
        if (typing) document.activeElement.blur();
        logsRef.current?.clearFilterAndSearch();
        return;
      }

      if (showHelp || typing) return;

      if (ev.key === "/") {
        ev.preventDefault();
        setActiveView("logs");
        // Wait a tick for the Logs view to actually be visible/focusable
        // (it's always mounted, but a display:none input can't receive
        // focus) before focusing it.
        requestAnimationFrame(() => logsRef.current?.focusSearch());
      } else if (ev.key === "h" || ev.key === "l" || ev.key === "t") {
        setActiveView({ h: "health", l: "logs", t: "topology" }[ev.key]);
      } else if ((ev.key === "n" || ev.key === "p") && activeView === "logs") {
        logsRef.current?.jumpToError(ev.key === "n" ? 1 : -1);
      }
    }

    document.addEventListener("keydown", onKeyDown);
    return () => document.removeEventListener("keydown", onKeyDown);
  }, [showHelp, activeView]);

  const connClass = connStatus.ok === true ? "ok" : connStatus.ok === false ? "err" : undefined;
  const connText = connStatus.ok === true ? "connected" : connStatus.ok === false ? `connection error: ${connStatus.detail}` : "connecting...";

  return (
    <>
      <header>
        <h1>Security Ginger</h1>
        <nav>
          {TABS.map((tab) => (
            <button
              key={tab.id}
              type="button"
              className={"tab-btn" + (activeView === tab.id ? " active" : "")}
              onClick={() => setActiveView(tab.id)}
            >
              {tab.label}
            </button>
          ))}
        </nav>
        <span className="help-hint">
          press <kbd>?</kbd> for shortcuts
        </span>
      </header>

      <main>
        <HealthView active={activeView === "health"} onConnStatus={onConnStatus} />
        <LogsView ref={logsRef} active={activeView === "logs"} onConnStatus={onConnStatus} />
        <TopologyView active={activeView === "topology"} onConnStatus={onConnStatus} />
      </main>

      <footer>
        <span id="conn-status" className={connClass}>
          {connText}
        </span>
      </footer>

      {showHelp && <HelpOverlay onClose={() => setShowHelp(false)} />}
    </>
  );
}
