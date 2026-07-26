package runlog

import (
	"context"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestWriterStoresJSONLinesAndFiltersByTag(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.jsonl")
	w, err := NewWriter(path)
	if err != nil {
		t.Fatalf("new writer: %v", err)
	}
	if err := w.Append("browser", "stdout", "chromium ready"); err != nil {
		t.Fatalf("append browser: %v", err)
	}
	if err := w.Append("server", "stderr", "server ready"); err != nil {
		t.Fatalf("append server: %v", err)
	}
	if err := w.Close(); err != nil {
		t.Fatalf("close: %v", err)
	}

	entries, err := ReadLast(path, 10, "chromium")
	if err != nil {
		t.Fatalf("read last: %v", err)
	}
	if len(entries) != 1 {
		t.Fatalf("entries len got %d want 1", len(entries))
	}
	if entries[0].Tag != "browser" {
		t.Fatalf("tag got %q want browser", entries[0].Tag)
	}

	line := Format(entries[0])
	if !strings.Contains(line, "[chromium] chromium ready") {
		t.Fatalf("formatted line got %q", line)
	}
}

func TestNormalizeFilterRejectsUnknownValues(t *testing.T) {
	if got, err := NormalizeFilter("chromium"); err != nil || got != "browser" {
		t.Fatalf("chromium filter got %q %v", got, err)
	}
	if got, err := NormalizeFilter("server"); err != nil || got != "server" {
		t.Fatalf("server filter got %q %v", got, err)
	}
	if got, err := NormalizeFilter("daemon"); err != nil || got != "daemon" {
		t.Fatalf("daemon filter got %q %v", got, err)
	}
	if _, err := NormalizeFilter("agent"); err == nil {
		t.Fatal("expected invalid filter error")
	}
}

func TestFollowFromStartReadsExistingEntries(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.jsonl")
	w, err := NewWriter(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := w.Append("daemon", "lifecycle", "building agent"); err != nil {
		t.Fatal(err)
	}
	if err := w.Close(); err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	entries := make(chan Entry, 1)
	errCh := make(chan error, 1)
	go func() {
		errCh <- FollowFromStartWithContext(ctx, path, "daemon", func(entry Entry) {
			entries <- entry
			cancel()
		})
	}()

	select {
	case entry := <-entries:
		if entry.Line != "building agent" {
			t.Fatalf("entry line got %q", entry.Line)
		}
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for existing entry")
	}
	if err := <-errCh; err != nil {
		t.Fatalf("follow from start: %v", err)
	}
}

func TestFollowReopensReplacedLogFile(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.jsonl")
	if err := os.WriteFile(path, []byte(""), 0644); err != nil {
		t.Fatal(err)
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	entries := make(chan Entry, 1)
	errCh := make(chan error, 1)
	go func() {
		errCh <- FollowWithContext(ctx, path, "", func(entry Entry) {
			entries <- entry
		})
	}()

	time.Sleep(20 * time.Millisecond)
	if err := os.Remove(path); err != nil {
		t.Fatal(err)
	}
	w, err := NewWriter(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := w.Append("server", "stdout", "after restart"); err != nil {
		t.Fatal(err)
	}
	if err := w.Close(); err != nil {
		t.Fatal(err)
	}

	select {
	case entry := <-entries:
		if entry.Line != "after restart" {
			t.Fatalf("entry line got %q", entry.Line)
		}
	case <-time.After(time.Second):
		t.Fatal("timed out waiting for followed entry")
	}
	cancel()
	if err := <-errCh; err != nil {
		t.Fatalf("follow: %v", err)
	}
}
