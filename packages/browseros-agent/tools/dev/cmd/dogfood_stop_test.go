package cmd

import (
	"bytes"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

func TestWaitForDogfoodStoppedWarnsWhenRunFileCleanupFails(t *testing.T) {
	root := t.TempDir()
	socketPath := filepath.Join(root, "dogfood.sock")
	if err := os.Mkdir(socketPath, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(socketPath, "child"), []byte("x"), 0o644); err != nil {
		t.Fatal(err)
	}

	var out bytes.Buffer
	stopped, err := waitForDogfoodStopped(&out, dogfoodRuntimeTarget{
		LockPath:   filepath.Join(root, "run.lock"),
		SocketPath: socketPath,
		StatePath:  filepath.Join(root, "state.json"),
	}, time.Millisecond)
	if err != nil {
		t.Fatal(err)
	}
	if !stopped {
		t.Fatal("expected inactive dogfood run to be treated as stopped")
	}
	if !strings.Contains(out.String(), "Warning:") {
		t.Fatalf("missing cleanup warning:\n%s", out.String())
	}
}
