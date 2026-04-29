package cmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"browseros-dogfood/config"
	dogfoodruntime "browseros-dogfood/runtime"
)

func TestAcquireRefreshProfileLockReportsStopCommandWhenRunning(t *testing.T) {
	paths := newRunPaths(filepath.Join(t.TempDir(), "config.yaml"))
	lock, err := dogfoodruntime.AcquireLock(paths.Lock)
	if err != nil {
		t.Fatal(err)
	}
	defer lock.Close()
	if err := dogfoodruntime.WriteRunState(paths.State, dogfoodruntime.RunState{
		PID:  12345,
		Mode: "background",
	}); err != nil {
		t.Fatal(err)
	}

	_, err = acquireRefreshProfileLock(paths)

	if err == nil {
		t.Fatal("expected refresh profile lock error")
	}
	for _, want := range []string{"cannot refresh profile", "browseros-dogfood stop", "12345"} {
		if !strings.Contains(err.Error(), want) {
			t.Fatalf("error missing %q: %v", want, err)
		}
	}
}

func TestEnsureDevProfileNotInUseReportsStopCommand(t *testing.T) {
	devUserDataDir := t.TempDir()
	if err := os.WriteFile(filepath.Join(devUserDataDir, "SingletonLock"), []byte("lock"), 0644); err != nil {
		t.Fatal(err)
	}

	err := ensureDevProfileNotInUse(config.Config{DevUserDataDir: devUserDataDir})

	if err == nil {
		t.Fatal("expected dev profile in-use error")
	}
	for _, want := range []string{"cannot refresh profile", "browseros-dogfood stop", devUserDataDir} {
		if !strings.Contains(err.Error(), want) {
			t.Fatalf("error missing %q: %v", want, err)
		}
	}
}
