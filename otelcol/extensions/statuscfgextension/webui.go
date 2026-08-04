package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"embed"
	"io/fs"
	"net/http"
)

// webUIFS holds the plain HTML/CSS/vanilla-JS single-page app served at "/"
// -- a health dashboard, log viewer, and topology diagram, all backed by the
// JSON endpoints already registered alongside it. Compiled into the binary,
// no Node/build step involved.
//
//go:embed webui/index.html webui/style.css webui/app.js
var webUIFS embed.FS

func webUIHandler() http.Handler {
	assets, err := fs.Sub(webUIFS, "webui")
	if err != nil {
		// Only possible if the embed directive above and this Sub path
		// disagree, which would be a build-time programming error, not a
		// runtime condition -- fail loudly rather than serve nothing.
		panic(err)
	}
	return http.FileServer(http.FS(assets))
}
