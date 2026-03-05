package e2e

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"testing"
)

var bdevBinary string

func TestMain(m *testing.M) {
	tmpDir, err := os.MkdirTemp("", "bdev-e2e-bin-*")
	if err != nil {
		fmt.Fprintf(os.Stderr, "failed to create temp dir: %v\n", err)
		os.Exit(1)
	}

	_, file, _, ok := runtime.Caller(0)
	if !ok {
		fmt.Fprintln(os.Stderr, "failed to resolve e2e test path")
		os.Exit(1)
	}

	moduleDir := filepath.Clean(filepath.Join(filepath.Dir(file), ".."))
	bdevBinary = filepath.Join(tmpDir, "bdev-e2e")

	build := exec.Command("go", "build", "-o", bdevBinary, ".")
	build.Dir = moduleDir
	if out, err := build.CombinedOutput(); err != nil {
		fmt.Fprintf(os.Stderr, "failed to build bdev binary: %v\n%s\n", err, string(out))
		os.Exit(1)
	}

	code := m.Run()
	_ = os.RemoveAll(tmpDir)
	os.Exit(code)
}
