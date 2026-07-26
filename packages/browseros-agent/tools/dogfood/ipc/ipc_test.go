package ipc

import (
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestServerHandlesRequest(t *testing.T) {
	socketPath := filepath.Join(t.TempDir(), "daemon.sock")
	server := NewServer(socketPath, HandlerFunc(func(req Request) Response {
		if req.Command != CmdStatus {
			return Response{Error: "wrong command"}
		}
		return Response{OK: true, Data: map[string]string{"state": "running"}}
	}))
	if err := server.Start(); err != nil {
		t.Fatalf("start server: %v", err)
	}
	defer server.Stop()

	resp, err := NewClient(socketPath).Send(Request{Command: CmdStatus})
	if err != nil {
		t.Fatalf("send: %v", err)
	}
	if !resp.OK {
		t.Fatalf("response got %#v", resp)
	}
	data := resp.Data.(map[string]any)
	if data["state"] != "running" {
		t.Fatalf("data got %#v", data)
	}
}

func TestClientReportsMissingDaemon(t *testing.T) {
	_, err := NewClient(filepath.Join(t.TempDir(), "missing.sock")).Send(Request{Command: CmdStatus})
	if err == nil {
		t.Fatal("expected missing daemon error")
	}
	if !errors.Is(err, ErrDaemonNotRunning) {
		t.Fatalf("error got %v want ErrDaemonNotRunning", err)
	}
	if strings.Contains(err.Error(), "browseros-dogfood start-background") {
		t.Fatalf("ipc layer should not include targetless CLI advice: %v", err)
	}
}

func TestClientSendTimesOutWhenDaemonDoesNotRespond(t *testing.T) {
	dir, err := os.MkdirTemp("", "ipc")
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = os.RemoveAll(dir) })
	socketPath := filepath.Join(dir, "daemon.sock")
	server := NewServer(socketPath, HandlerFunc(func(req Request) Response {
		time.Sleep(50 * time.Millisecond)
		return Response{OK: true}
	}))
	if err := server.Start(); err != nil {
		t.Fatalf("start server: %v", err)
	}
	defer server.Stop()

	_, err = NewClientWithTimeout(socketPath, 5*time.Millisecond).Send(Request{Command: CmdStatus})
	if err == nil {
		t.Fatal("expected response timeout")
	}
}
