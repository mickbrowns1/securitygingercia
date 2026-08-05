# webui-react

The embedded web UI's React source -- Health, Logs, and Topology/Sankey
views, all polling the same `/status`/`/logs`/`/topology` endpoints
`../webui.go` registers alongside it. This retired an earlier plain
HTML/CSS/vanilla-JS version (same three views, same behavior) -- see
`../webui.go`'s doc comment for why Node is now a build-time
dependency of this distribution.

## Building

`install.sh` bootstraps Node and runs this for you, the same way it
already bootstraps Go/Rust -- see its "Checking for Node.js" and
"Building the web UI" steps. Building by hand (e.g. after editing
something here) is the same thing manually:

```bash
npm ci
npm run build
```

Produces `dist/`, which `../webui.go`'s `//go:embed webui-react/dist`
directive expects to exist *before* `go build`/the OCB `builder`
command runs -- `dist/` is gitignored (build output, not source), so
this step is required after any source change here, before rebuilding
`sgcia-otelcol`.

## Structure

- `src/App.jsx` -- header/tabs, footer connection status, global
  keyboard shortcut handling (`h`/`l`/`t` tab-jump, `/` focus search,
  `n`/`p` error-jump, `?` help, `Esc` clear-filter-or-close-help).
- `src/views/{Health,Logs,Topology}View.jsx` -- one component per tab,
  each owns its own polling (`useInterval`, only while its tab is
  active) and connection-status reporting.
- `src/sankey.js` -- the Sankey diagram's layout math, kept separate
  from JSX rendering (`views/TopologyView.jsx` renders its output) --
  sizes every node/edge off `events_in`, never `events_out` (a
  pipeline's `events_out` is the *sum* of what it sent to every
  exporter it feeds, which would inflate its own node and each
  outbound edge by however many exporters it happens to feed), colors
  each pipeline consistently across crossings with per-receiver shade
  variants on inbound edges, and shows a percentage of total ingested
  logs alongside every raw count.
- `src/index.css` -- the UI's stylesheet (CSS translates 1:1
  regardless of framework, so this didn't need a React-specific
  rewrite).

All three views stay mounted at all times (CSS `display:none` on the
inactive ones via `.view`/`.view.active`) rather than being
conditionally unmounted -- this keeps a couple of intentionally subtle
behaviors (e.g. pressing `/` from a non-Logs tab can't focus a
display:none input) and avoids losing each view's own state on every
tab switch.
