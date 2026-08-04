package statuscfgextension

import "testing"

// TestComputePipelineSnapshots_ZeroTrafficPipelineDoesNotInheritSharedExportersTotal
// reproduces the exact bug hit live: three pipelines all wired to the same
// two exporters, only two of them actually receiving traffic. The naive
// sum-per-exporter-per-pipeline approach gave the idle third pipeline the
// exporters' *entire* combined total (420) despite it having 0 events_in.
func TestComputePipelineSnapshots_ZeroTrafficPipelineDoesNotInheritSharedExportersTotal(t *testing.T) {
	pipelines := map[string]pipelineTopology{
		"logs/syslog":     {Receivers: []string{"syslog/udp"}, Exporters: []string{"splunk_hec/sentinelone", "logbuffer"}},
		"logs/syslog5424": {Receivers: []string{"syslog/rfc5424"}, Exporters: []string{"splunk_hec/sentinelone", "logbuffer"}},
		"logs/files":      {Receivers: []string{"file_log/app"}, Exporters: []string{"splunk_hec/sentinelone", "logbuffer"}},
	}
	acceptedLog := map[string]uint64{"syslog/udp": 60, "syslog/rfc5424": 150, "file_log/app": 0}
	sentLog := map[string]uint64{"splunk_hec/sentinelone": 210, "logbuffer": 210}
	failedLog := map[string]uint64{}

	got := computePipelineSnapshots(pipelines, acceptedLog, sentLog, failedLog)

	if got["logs/files"].EventsIn != 0 {
		t.Fatalf("logs/files EventsIn = %d, want 0", got["logs/files"].EventsIn)
	}
	if got["logs/files"].EventsOut != 0 {
		t.Errorf("logs/files EventsOut = %d, want 0 -- it received no traffic, it shouldn't inherit the shared exporters' full total", got["logs/files"].EventsOut)
	}

	// Both exporters are shared by the same two active pipelines, each
	// independently reporting 210 sent -- so each pipeline's share is
	// proportional to its own events_in (60/210 and 150/210) from *each*
	// exporter, summing to 120 and 300 respectively. Neither independently
	// claims the full 420 (both exporters' totals combined).
	wantSyslog := uint64(120)
	wantSyslog5424 := uint64(300)
	if got["logs/syslog"].EventsOut != wantSyslog {
		t.Errorf("logs/syslog EventsOut = %d, want %d", got["logs/syslog"].EventsOut, wantSyslog)
	}
	if got["logs/syslog5424"].EventsOut != wantSyslog5424 {
		t.Errorf("logs/syslog5424 EventsOut = %d, want %d", got["logs/syslog5424"].EventsOut, wantSyslog5424)
	}
}

func TestComputePipelineSnapshots_ExporterUsedByExactlyOnePipelineIsExact(t *testing.T) {
	pipelines := map[string]pipelineTopology{
		"logs/only": {Receivers: []string{"syslog/udp"}, Exporters: []string{"splunk_hec/sentinelone"}},
	}
	acceptedLog := map[string]uint64{"syslog/udp": 42}
	sentLog := map[string]uint64{"splunk_hec/sentinelone": 42}
	failedLog := map[string]uint64{"splunk_hec/sentinelone": 3}

	got := computePipelineSnapshots(pipelines, acceptedLog, sentLog, failedLog)

	if got["logs/only"].EventsOut != 42 || got["logs/only"].EventsDropped != 3 {
		t.Fatalf("got %+v, want EventsOut=42 EventsDropped=3 (no sharing, so no approximation needed)", got["logs/only"])
	}
}

func TestComputePipelineSnapshots_FallsBackToEvenSplitWhenNoPipelineHasAnyEventsIn(t *testing.T) {
	pipelines := map[string]pipelineTopology{
		"logs/a": {Receivers: []string{"r-a"}, Exporters: []string{"shared"}},
		"logs/b": {Receivers: []string{"r-b"}, Exporters: []string{"shared"}},
	}
	acceptedLog := map[string]uint64{"r-a": 0, "r-b": 0}
	sentLog := map[string]uint64{"shared": 100}
	failedLog := map[string]uint64{}

	got := computePipelineSnapshots(pipelines, acceptedLog, sentLog, failedLog)

	if got["logs/a"].EventsOut != 50 || got["logs/b"].EventsOut != 50 {
		t.Fatalf("got a=%d b=%d, want an even 50/50 split when neither pipeline has any events_in to weight by",
			got["logs/a"].EventsOut, got["logs/b"].EventsOut)
	}
}
