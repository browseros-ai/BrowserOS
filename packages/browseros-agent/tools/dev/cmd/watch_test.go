package cmd

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestEnsureDevCachePresentMissingMessage(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)

	err := ensureDevCachePresent()
	if err == nil {
		t.Fatal("expected missing cache error")
	}

	msg := err.Error()
	if !strings.Contains(msg, "VM cache is missing.") {
		t.Fatalf("expected missing cache message, got %q", msg)
	}
	if strings.Count(msg, "dev:setup") != 1 {
		t.Fatalf("expected dev:setup once, got %q", msg)
	}
	if strings.Contains(msg, home) {
		t.Fatalf("expected cache path to be hidden, got %q", msg)
	}
}

func TestEnsureLimactlPresentMissingMessage(t *testing.T) {
	t.Setenv("PATH", t.TempDir())

	err := ensureLimactlPresent()
	if err == nil {
		t.Fatal("expected missing Lima error")
	}

	msg := err.Error()
	if !strings.Contains(msg, "Lima is not installed.") {
		t.Fatalf("expected missing Lima message, got %q", msg)
	}
	if !strings.Contains(msg, "brew install lima") {
		t.Fatalf("expected brew install hint, got %q", msg)
	}
}

func TestEnsureLimactlPresentFindsPathBinary(t *testing.T) {
	binDir := t.TempDir()
	limactlPath := filepath.Join(binDir, "limactl")
	if err := os.WriteFile(limactlPath, []byte("#!/bin/sh\n"), 0o755); err != nil {
		t.Fatal(err)
	}
	t.Setenv("PATH", binDir)

	if err := ensureLimactlPresent(); err != nil {
		t.Fatalf("expected limactl to resolve, got %v", err)
	}
}
