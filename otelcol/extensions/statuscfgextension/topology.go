package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

// topologyNode/topologyEdge back GET /topology -- a config-derived
// analogue of a distributed-tracing "service map," built from
// service.pipelines instead of live span data (this collector has no
// tracing signal at all, only logs, so there's no runtime call graph to
// draw from).
type topologyNode struct {
	ID   string `json:"id"`
	Type string `json:"type"` // "receiver" | "exporter" | "pipeline"
}

type topologyEdge struct {
	From string `json:"from"`
	To   string `json:"to"`
}

type topologyGraph struct {
	Nodes []topologyNode `json:"nodes"`
	Edges []topologyEdge `json:"edges"`
}

// buildTopology lists every configured receiver/exporter (even ones no
// pipeline currently references -- an orphaned component is worth
// surfacing, not hiding) plus one node per pipeline, with edges for
// each pipeline's actual receiver/exporter membership.
func (rc *resolvedConfig) buildTopology() topologyGraph {
	var g topologyGraph

	for _, id := range rc.receiverIDs {
		g.Nodes = append(g.Nodes, topologyNode{ID: id, Type: "receiver"})
	}
	for _, id := range rc.exporterIDs {
		g.Nodes = append(g.Nodes, topologyNode{ID: id, Type: "exporter"})
	}
	for name, topo := range rc.pipelines {
		g.Nodes = append(g.Nodes, topologyNode{ID: name, Type: "pipeline"})
		for _, r := range topo.Receivers {
			g.Edges = append(g.Edges, topologyEdge{From: r, To: name})
		}
		for _, x := range topo.Exporters {
			g.Edges = append(g.Edges, topologyEdge{From: name, To: x})
		}
	}
	return g
}
