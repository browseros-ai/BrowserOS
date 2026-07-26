package cmd

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"browseros-dogfood/config"
)

func TestPrintLogsShowsDirectoryAndFiles(t *testing.T) {
	devDir := t.TempDir()
	cfg := config.Config{DevUserDataDir: devDir}
	logDir := cfg.LogDir()
	if err := os.MkdirAll(logDir, 0755); err != nil {
		t.Fatal(err)
	}
	for _, name := range []string{"server.log", "chromium.log"} {
		if err := os.WriteFile(filepath.Join(logDir, name), []byte("log"), 0644); err != nil {
			t.Fatal(err)
		}
	}

	var out bytes.Buffer
	if err := printLogs(&out, cfg); err != nil {
		t.Fatal(err)
	}

	got := stripANSI(out.String())
	for _, want := range []string{
		"Log directory: " + logDir,
		filepath.Join(logDir, "chromium.log"),
		filepath.Join(logDir, "server.log"),
	} {
		if !strings.Contains(got, want) {
			t.Fatalf("missing %q in\n%s", want, got)
		}
	}
}

func TestPrintLogsHandlesMissingDirectory(t *testing.T) {
	cfg := config.Config{DevUserDataDir: t.TempDir()}

	var out bytes.Buffer
	if err := printLogs(&out, cfg); err != nil {
		t.Fatal(err)
	}

	got := stripANSI(out.String())
	if !strings.Contains(got, "No log files found.") {
		t.Fatalf("unexpected output:\n%s", got)
	}
}
