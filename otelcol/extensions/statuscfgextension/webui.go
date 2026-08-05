package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"embed"
	"io/fs"
	"net/http"
)

// webUIFS holds the React single-page app served at "/" -- a health
// dashboard, log viewer, and topology diagram, all backed by the JSON
// endpoints already registered alongside it. Requires `npm run build`
// in webui-react/ before this package builds, since it embeds that
// build's dist/ output rather than flat source files -- `install.sh`
// does this for you (bootstrapping Node the same way it already does
// Go/Rust); see webui-react/README.md for the manual equivalent.
//
//go:embed webui-react/dist
var webUIFS embed.FS

func webUIHandler() http.Handler {
	assets, err := fs.Sub(webUIFS, "webui-react/dist")
	if err != nil {
		// Only possible if the embed directive above and this Sub path
		// disagree, which would be a build-time programming error, not a
		// runtime condition -- fail loudly rather than serve nothing.
		panic(err)
	}
	return http.FileServer(http.FS(assets))
}
