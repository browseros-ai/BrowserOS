package git

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestRunReturnsContextError(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	config := []byte("[alias]\n\thold = !sleep 5\n")
	if err := os.WriteFile(filepath.Join(home, ".gitconfig"), config, 0o644); err != nil {
		t.Fatalf("WriteFile: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Millisecond)
	defer cancel()

	if _, err := Run(ctx, t.TempDir(), nil, "hold"); err == nil {
		t.Fatalf("expected timeout error")
	}
	if ctx.Err() != context.DeadlineExceeded {
		t.Fatalf("expected context deadline exceeded, got %v", ctx.Err())
	}
}
