package cmd

import (
	"context"
	"errors"
	"fmt"
	"net"
	"net/http"
	"path/filepath"
	"reflect"
	"testing"
	"time"

	"browseros-dogfood/config"
	"browseros-dogfood/runlog"
)

func TestRunPathsLiveBesideConfig(t *testing.T) {
	paths := newTargetRunPaths(filepath.Join("/tmp", "browseros-dogfood", "config.yaml"), config.TargetClaw)
	if paths.Lock != filepath.Join("/tmp", "browseros-dogfood", "claw", "run.lock") {
		t.Fatalf("lock path got %q", paths.Lock)
	}
	if paths.Socket != filepath.Join("/tmp", "browseros-dogfood", "claw", "daemon.sock") {
		t.Fatalf("socket path got %q", paths.Socket)
	}
	if paths.Log != filepath.Join("/tmp", "browseros-dogfood", "claw", "daemon.jsonl") {
		t.Fatalf("log path got %q", paths.Log)
	}
}

func TestDaemonArgsIncludeHeadlessWhenRequested(t *testing.T) {
	got := daemonArgs(config.TargetClaw, true)
	want := []string{"--claw", "daemon", "--headless"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("got %#v want %#v", got, want)
	}
}

func TestDaemonStatusIncludesBrowserOSDir(t *testing.T) {
	d := &dogfoodDaemon{
		state:        "running",
		startedAt:    time.Now(),
		browserOSDir: "/tmp/browseros-dogfood",
	}

	got := d.status()

	if got.BrowserOSDir != "/tmp/browseros-dogfood" {
		t.Fatalf("BrowserOS dir got %q", got.BrowserOSDir)
	}
}

func TestLogLifecycleWritesDaemonRunlogEntry(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.jsonl")
	writer, err := runlog.NewWriter(path)
	if err != nil {
		t.Fatal(err)
	}
	d := &dogfoodDaemon{logWriter: writer}

	d.logLifecycle("building agent")
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}

	entries, err := runlog.ReadLast(path, 10, "daemon")
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 {
		t.Fatalf("entries len got %d want 1", len(entries))
	}
	if entries[0].Line != "building agent" {
		t.Fatalf("entry line got %q", entries[0].Line)
	}
}

func TestWaitForServerHealthRequiresCDPConnectedWhenPresent(t *testing.T) {
	var requests int
	port, shutdown := startHealthTestServer(t, func(w http.ResponseWriter, r *http.Request) {
		requests++
		if requests == 1 {
			fmt.Fprint(w, `{"status":"ok","cdpConnected":false}`)
			return
		}
		fmt.Fprint(w, `{"status":"ok","cdpConnected":true}`)
	})
	defer shutdown()

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if err := waitForServerHealth(ctx, config.Config{Ports: config.Ports{Server: port}}, 5, 10*time.Millisecond); err != nil {
		t.Fatalf("wait health: %v", err)
	}
	if requests < 2 {
		t.Fatalf("requests got %d want at least 2", requests)
	}
}

func TestWaitForServerHealthAcceptsMissingCDPConnected(t *testing.T) {
	port, shutdown := startHealthTestServer(t, func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"status":"ok"}`)
	})
	defer shutdown()

	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()
	if err := waitForServerHealth(ctx, config.Config{Ports: config.Ports{Server: port}}, 1, 10*time.Millisecond); err != nil {
		t.Fatalf("wait health: %v", err)
	}
}

func TestHealthURLUsesSystemHealth(t *testing.T) {
	got := healthURL(config.Config{
		Ports: config.Ports{Server: 9200},
	})
	if got != "http://127.0.0.1:9200/system/health" {
		t.Fatalf("got %q", got)
	}
}

func TestWaitUntilHealthyLogsHealthLifecycle(t *testing.T) {
	port, shutdown := startHealthTestServer(t, func(w http.ResponseWriter, r *http.Request) {
		fmt.Fprint(w, `{"status":"ok","cdpConnected":true}`)
	})
	defer shutdown()

	path := filepath.Join(t.TempDir(), "daemon.jsonl")
	writer, err := runlog.NewWriter(path)
	if err != nil {
		t.Fatal(err)
	}
	d := &dogfoodDaemon{
		ctx:       context.Background(),
		logWriter: writer,
	}
	if err := d.waitUntilHealthy(config.Config{Ports: config.Ports{Server: port}}, 1, 10*time.Millisecond); err != nil {
		t.Fatalf("wait healthy: %v", err)
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}

	entries, err := runlog.ReadLast(path, 10, "daemon")
	if err != nil {
		t.Fatal(err)
	}
	got := make([]string, 0, len(entries))
	for _, entry := range entries {
		got = append(got, entry.Line)
	}
	want := []string{"waiting for server health", "server healthy"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("lifecycle entries got %#v want %#v", got, want)
	}
}

func TestWithOperationLogsFailureLifecycle(t *testing.T) {
	path := filepath.Join(t.TempDir(), "daemon.jsonl")
	writer, err := runlog.NewWriter(path)
	if err != nil {
		t.Fatal(err)
	}
	d := &dogfoodDaemon{logWriter: writer}
	err = d.withOperation("starting", func() error {
		return errors.New("boom")
	})
	if err == nil {
		t.Fatal("expected operation error")
	}
	if err := writer.Close(); err != nil {
		t.Fatal(err)
	}

	entries, err := runlog.ReadLast(path, 10, "daemon")
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 || entries[0].Line != "starting failed: boom" {
		t.Fatalf("entries got %#v", entries)
	}
}

func startHealthTestServer(t *testing.T, handler http.HandlerFunc) (int, func()) {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	server := &http.Server{Handler: handler}
	go func() {
		_ = server.Serve(listener)
	}()
	return listener.Addr().(*net.TCPAddr).Port, func() {
		_ = server.Close()
	}
}
