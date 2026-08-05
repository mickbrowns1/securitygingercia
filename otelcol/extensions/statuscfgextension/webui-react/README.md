# webui-react (experimental -- branch `reactdontdoit`)

A React port of the plain HTML/CSS/vanilla-JS web UI at
`../webui/` (Health, Logs, and Topology/Sankey views, all polling the
same `/status`/`/logs`/`/topology` endpoints). This exists to answer
"how hard would this be" concretely, not because it's decided this is
replacing the vanilla version -- see `../webui.go`'s doc comment and
the `reactdontdoit` branch itself.

## Building

```bash
npm install
npm run build
```

Produces `dist/`, which `../webui.go`'s `//go:embed webui-react/dist`
directive expects to exist *before* `go build`/the OCB `builder`
command runs -- `dist/` is gitignored (build output, not source), so
this step is required after every fresh clone of this branch, and
after every source change here, before rebuilding `sgcia-otelcol`.
This is exactly the "Node becomes a build-time dependency" tradeoff
discussed when this branch was proposed -- `install.sh` doesn't wire
this in yet, since that's a real decision for whoever decides whether
to actually adopt this, not something to default into on an
experimental branch.

## Structure

- `src/App.jsx` -- header/tabs, footer connection status, global
  keyboard shortcut handling (mirrors the original's shortcuts
  exactly: `h`/`l`/`t` tab-jump, `/` focus search, `n`/`p` error-jump,
  `?` help, `Esc` clear-filter-or-close-help).
- `src/views/{Health,Logs,Topology}View.jsx` -- one component per tab,
  each owns its own polling (`useInterval`, only while its tab is
  active) and connection-status reporting.
- `src/sankey.js` -- the Sankey diagram's layout math, kept separate
  from JSX rendering (`views/TopologyView.jsx` renders its output) --
  ported 1:1 from the vanilla version's `app.js`, including the
  events_in-not-events_out sizing, per-pipeline/per-receiver
  color-and-shade logic, and percentage-of-total labels.
- `src/index.css` -- the original `style.css`, copied over unchanged
  (CSS translates 1:1 regardless of framework).

All three views stay mounted at all times (CSS `display:none` on the
inactive ones, via the original's `.view`/`.view.active` classes)
rather than being conditionally unmounted -- this preserves a couple
of the original's subtler behaviors (e.g. pressing `/` from a
non-Logs tab can't focus a display:none input, matching the original
exactly) and avoids losing each view's own state on every tab switch.
