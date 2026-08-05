package main

import (
	"embed"
	"io/fs"
	"net/http"
)

// webUIFS holds the small Fleet React app served at "/" -- an agent
// inventory list with a health rollup, deep-linking into each agent's own
// existing per-agent UI for drill-down. Deliberately a separate app from
// otelcol/extensions/statuscfgextension/webui-react rather than a shared
// build: this binary's API (/agents, /agents/{id}) has nothing in common
// with a single agent's (/status, /topology, /logs), so sharing one React
// app would mean a mode flag and conditional tabs for no real benefit.
// Requires `npm run build` in webui/ before this package builds, since it
// embeds that build's dist/ output.
//
//go:embed webui/dist
var webUIFS embed.FS

func webUIHandler() http.Handler {
	assets, err := fs.Sub(webUIFS, "webui/dist")
	if err != nil {
		panic(err)
	}
	return http.FileServer(http.FS(assets))
}
