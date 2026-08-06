package statuscfgextension // import "github.com/mickbrowns1/securitygingercia/otelcol/extensions/statuscfgextension"

import (
	"os"
	"path/filepath"
	"testing"

	"go.uber.org/zap"
)

func TestLoadOrCreateInstanceUID_PersistsAcrossCalls(t *testing.T) {
	path := filepath.Join(t.TempDir(), "nested", "instance-uid")
	logger := zap.NewNop()

	first := loadOrCreateInstanceUID(path, logger)
	second := loadOrCreateInstanceUID(path, logger)

	if first != second {
		t.Fatalf("expected the same instance UID across calls (simulating a restart), got %x then %x", first, second)
	}
}

func TestLoadOrCreateInstanceUID_EmptyPathIsNotPersistent(t *testing.T) {
	logger := zap.NewNop()

	first := loadOrCreateInstanceUID("", logger)
	second := loadOrCreateInstanceUID("", logger)

	if first == second {
		t.Fatalf("expected different instance UIDs when persistence is disabled (empty path), got the same value twice: %x", first)
	}
}

func TestLoadOrCreateInstanceUID_CorruptFileRegeneratesRatherThanFails(t *testing.T) {
	path := filepath.Join(t.TempDir(), "instance-uid")
	if err := os.WriteFile(path, []byte("not-valid-hex"), 0o600); err != nil {
		t.Fatal(err)
	}

	got := loadOrCreateInstanceUID(path, zap.NewNop())
	var zero [16]byte
	if got == zero {
		t.Fatal("expected a real generated UID, not the zero value")
	}
}

func TestDecodeInstanceUID_RejectsWrongLength(t *testing.T) {
	if _, err := decodeInstanceUID("abcd"); err == nil {
		t.Fatal("expected an error for a too-short hex string")
	}
	if _, err := decodeInstanceUID("not hex at all!!"); err == nil {
		t.Fatal("expected an error for non-hex content")
	}
}
