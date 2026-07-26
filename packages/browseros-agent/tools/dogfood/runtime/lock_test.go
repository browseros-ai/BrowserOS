package runtime

import (
	"errors"
	"os"
	"path/filepath"
	"testing"
)

func TestLockExcludesSecondOwnerAndReleasesOnClose(t *testing.T) {
	path := filepath.Join(t.TempDir(), "run.lock")

	first, err := AcquireLock(path)
	if err != nil {
		t.Fatalf("first acquire: %v", err)
	}

	if _, err := AcquireLock(path); !errors.Is(err, ErrAlreadyRunning) {
		t.Fatalf("second acquire got %v want ErrAlreadyRunning", err)
	}

	if err := first.Close(); err != nil {
		t.Fatalf("close first lock: %v", err)
	}

	second, err := AcquireLock(path)
	if err != nil {
		t.Fatalf("second acquire after close: %v", err)
	}
	if err := second.Close(); err != nil {
		t.Fatalf("close second lock: %v", err)
	}
}

func TestRunStateRemovesStaleSocketWhenLockIsAcquired(t *testing.T) {
	dir := t.TempDir()
	socketPath := filepath.Join(dir, "daemon.sock")
	statePath := filepath.Join(dir, "state.json")
	if err := WriteRunState(statePath, RunState{
		PID:        12345,
		Mode:       "background",
		SocketPath: socketPath,
	}); err != nil {
		t.Fatalf("write state: %v", err)
	}
	if err := touch(socketPath); err != nil {
		t.Fatalf("touch socket: %v", err)
	}

	if err := CleanupStaleRunFiles(statePath); err != nil {
		t.Fatalf("cleanup: %v", err)
	}
	if exists(socketPath) {
		t.Fatalf("stale socket still exists")
	}
	if exists(statePath) {
		t.Fatalf("stale state still exists")
	}
}

func touch(path string) error {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY, 0644)
	if err != nil {
		return err
	}
	return file.Close()
}

func exists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}
