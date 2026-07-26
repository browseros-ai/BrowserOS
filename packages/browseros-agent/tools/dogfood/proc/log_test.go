package proc

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"
)

func TestOpenLogFileRotatesFileOlderThanOneDay(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "server.log")
	if err := os.WriteFile(path, []byte("old\n"), 0644); err != nil {
		t.Fatal(err)
	}
	now := time.Date(2026, 4, 27, 12, 0, 0, 0, time.UTC)
	oldTime := now.Add(-25 * time.Hour)
	if err := os.Chtimes(path, oldTime, oldTime); err != nil {
		t.Fatal(err)
	}

	file, gotPath, err := OpenLogFile(dir, "server.log", now)
	if err != nil {
		t.Fatal(err)
	}
	if gotPath != path {
		t.Fatalf("got %q want %q", gotPath, path)
	}
	if _, err := file.WriteString("new\n"); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}

	rotated, err := os.ReadFile(path + ".old")
	if err != nil {
		t.Fatal(err)
	}
	if string(rotated) != "old\n" {
		t.Fatalf("rotated content = %q", rotated)
	}
	current, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(current) != "new\n" {
		t.Fatalf("current content = %q", current)
	}
}

func TestOpenLogFileAppendsFreshFile(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "chromium.log")
	if err := os.WriteFile(path, []byte("old\n"), 0644); err != nil {
		t.Fatal(err)
	}
	now := time.Date(2026, 4, 27, 12, 0, 0, 0, time.UTC)
	fresh := now.Add(-time.Hour)
	if err := os.Chtimes(path, fresh, fresh); err != nil {
		t.Fatal(err)
	}

	file, _, err := OpenLogFile(dir, "chromium.log", now)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := file.WriteString("new\n"); err != nil {
		t.Fatal(err)
	}
	if err := file.Close(); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(path + ".old"); !os.IsNotExist(err) {
		t.Fatalf("unexpected rotated file, stat err=%v", err)
	}
	current, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if string(current) != "old\nnew\n" {
		t.Fatalf("current content = %q", current)
	}
}

func TestListLogFilesReturnsRegularFilesSortedByName(t *testing.T) {
	dir := t.TempDir()
	for _, name := range []string{"server.log", "chromium.log.old", "chromium.log", "server.log.backup"} {
		if err := os.WriteFile(filepath.Join(dir, name), []byte(name), 0644); err != nil {
			t.Fatal(err)
		}
	}
	if err := os.Mkdir(filepath.Join(dir, "nested.log"), 0755); err != nil {
		t.Fatal(err)
	}

	files, err := ListLogFiles(dir)
	if err != nil {
		t.Fatal(err)
	}
	got := []string{}
	for _, file := range files {
		got = append(got, file.Name)
	}
	want := []string{"chromium.log", "chromium.log.old", "server.log"}
	if strings.Join(got, ",") != strings.Join(want, ",") {
		t.Fatalf("got %#v want %#v", got, want)
	}
}

func TestStreamLinesWritesTerminalAndFile(t *testing.T) {
	var terminal bytes.Buffer
	var file bytes.Buffer
	var mu sync.Mutex

	streamLines(strings.NewReader("first\nsecond\n"), TagServer, &terminal, &file, &mu)

	terminalOutput := terminal.String()
	if !strings.Contains(terminalOutput, "[server] first") || !strings.Contains(terminalOutput, "[server] second") {
		t.Fatalf("unexpected terminal output: %q", terminalOutput)
	}
	fileOutput := file.String()
	if fileOutput != "[server] first\n[server] second\n" {
		t.Fatalf("unexpected file output: %q", fileOutput)
	}
}

func TestStreamLinesLogsScannerErrors(t *testing.T) {
	var terminal bytes.Buffer
	var file bytes.Buffer
	var mu sync.Mutex
	longLine := strings.Repeat("x", 1024*1024+1)

	streamLines(strings.NewReader(longLine), TagBrowser, &terminal, &file, &mu)

	for name, got := range map[string]string{
		"terminal": terminal.String(),
		"file":     file.String(),
	} {
		if !strings.Contains(got, "log stream error: bufio.Scanner: token too long") {
			t.Fatalf("%s output missing scanner error: %q", name, got)
		}
	}
}

func TestStreamLinesWithHandlerSkipsEmptyLinesAndReportsStream(t *testing.T) {
	var got []string
	StreamLinesWithHandler(strings.NewReader("one\n\nthree\n"), TagServer, "stderr", func(tag Tag, stream string, line string) {
		got = append(got, tag.Name+":"+stream+":"+line)
	})

	want := []string{"server:stderr:one", "server:stderr:three"}
	if len(got) != len(want) {
		t.Fatalf("got %#v want %#v", got, want)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("entry %d got %q want %q", i, got[i], want[i])
		}
	}
}
