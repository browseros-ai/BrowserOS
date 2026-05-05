package workspace

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestConfigRoundTrip(t *testing.T) {
	configHome := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configHome)

	cfg := &Config{Version: 1, PatchesRepo: "/tmp/browseros"}
	if err := SaveConfig(cfg); err != nil {
		t.Fatalf("SaveConfig: %v", err)
	}

	loaded, err := LoadConfig()
	if err != nil {
		t.Fatalf("LoadConfig: %v", err)
	}
	if loaded.PatchesRepo != cfg.PatchesRepo {
		t.Fatalf("patches repo mismatch: got %q want %q", loaded.PatchesRepo, cfg.PatchesRepo)
	}
}

func TestRegistryDetectsLongestMatchingWorkspace(t *testing.T) {
	configHome := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configHome)

	root := t.TempDir()
	parent := filepath.Join(root, "chromium")
	child := filepath.Join(parent, "src")
	for _, dir := range []string{parent, child} {
		if err := os.MkdirAll(filepath.Join(dir, ".git"), 0o755); err != nil {
			t.Fatalf("mkdir: %v", err)
		}
	}
	detectedPath := filepath.Join(child, "chrome", "browser")
	if err := os.MkdirAll(detectedPath, 0o755); err != nil {
		t.Fatalf("mkdir detected path: %v", err)
	}

	reg := &Registry{Version: 1}
	if _, err := reg.Add("parent", parent); err != nil {
		t.Fatalf("add parent: %v", err)
	}
	if _, err := reg.Add("child", child); err != nil {
		t.Fatalf("add child: %v", err)
	}

	ws, err := Detect(reg, detectedPath)
	if err != nil {
		t.Fatalf("Detect: %v", err)
	}
	if ws.Name != "child" {
		t.Fatalf("expected child workspace, got %q", ws.Name)
	}
}

func TestDetectMatchesSymlinkedWorkingDirectory(t *testing.T) {
	root := t.TempDir()
	workspacePath := filepath.Join(root, "chromium-1", "src")
	if err := os.MkdirAll(filepath.Join(workspacePath, ".git"), 0o755); err != nil {
		t.Fatalf("mkdir workspace: %v", err)
	}
	if err := os.MkdirAll(filepath.Join(workspacePath, "chrome", "browser"), 0o755); err != nil {
		t.Fatalf("mkdir workspace child: %v", err)
	}
	linkPath := filepath.Join(root, "ch-1")
	if err := os.Symlink(workspacePath, linkPath); err != nil {
		t.Fatalf("symlink workspace: %v", err)
	}

	reg := &Registry{Version: 1}
	if _, err := reg.Add("ch1", workspacePath); err != nil {
		t.Fatalf("add workspace: %v", err)
	}

	ws, err := Detect(reg, filepath.Join(linkPath, "chrome", "browser"))
	if err != nil {
		t.Fatalf("Detect: %v", err)
	}
	if ws.Name != "ch1" {
		t.Fatalf("expected ch1 workspace, got %q", ws.Name)
	}
}

func TestRegistryAddStoresCanonicalWorkspacePath(t *testing.T) {
	root := t.TempDir()
	workspacePath := filepath.Join(root, "chromium-1", "src")
	if err := os.MkdirAll(filepath.Join(workspacePath, ".git"), 0o755); err != nil {
		t.Fatalf("mkdir workspace: %v", err)
	}
	linkPath := filepath.Join(root, "ch-1")
	if err := os.Symlink(workspacePath, linkPath); err != nil {
		t.Fatalf("symlink workspace: %v", err)
	}

	reg := &Registry{Version: 1}
	entry, err := reg.Add("ch1", linkPath)
	if err != nil {
		t.Fatalf("add workspace: %v", err)
	}
	expectedPath := canonicalPath(workspacePath)
	if entry.Path != expectedPath {
		t.Fatalf("expected canonical path %q, got %q", expectedPath, entry.Path)
	}
}

func TestDetectNoRegisteredCheckoutsError(t *testing.T) {
	_, err := Detect(&Registry{Version: 1}, t.TempDir())
	if err == nil {
		t.Fatalf("expected error")
	}

	message := err.Error()
	for _, want := range []string{
		"no Chromium checkouts registered",
		`browseros-patch add <name> <path>`,
	} {
		if !strings.Contains(message, want) {
			t.Fatalf("expected error to contain %q, got:\n%s", want, message)
		}
	}
}

func TestDetectOutsideRegisteredCheckoutError(t *testing.T) {
	root := t.TempDir()
	registered := filepath.Join(root, "chromium-src")
	outside := filepath.Join(root, "other")
	for _, dir := range []string{registered, outside} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatalf("mkdir: %v", err)
		}
	}

	reg := &Registry{Version: 1, Workspaces: []Entry{{Name: "ch1", Path: registered}}}
	_, err := Detect(reg, outside)
	if err == nil {
		t.Fatalf("expected error")
	}

	message := err.Error()
	for _, want := range []string{
		"not inside a registered Chromium checkout",
		"cwd: " + outside,
		"registered checkouts:",
		"ch1",
		registered,
		`browseros-patch diff ch1`,
	} {
		if !strings.Contains(message, want) {
			t.Fatalf("expected error to contain %q, got:\n%s", want, message)
		}
	}
	if strings.Contains(message, "sync state") {
		t.Fatalf("error should not imply list computes sync state:\n%s", message)
	}
}

func TestDetectOutsideRegisteredCheckoutErrorShowsResolvedCWD(t *testing.T) {
	root := t.TempDir()
	registered := filepath.Join(root, "chromium-src")
	realOutside := filepath.Join(root, "real-outside")
	linkOutside := filepath.Join(root, "link-outside")
	for _, dir := range []string{registered, realOutside} {
		if err := os.MkdirAll(dir, 0o755); err != nil {
			t.Fatalf("mkdir: %v", err)
		}
	}
	if err := os.Symlink(realOutside, linkOutside); err != nil {
		t.Fatalf("symlink: %v", err)
	}
	resolvedOutside, err := filepath.EvalSymlinks(realOutside)
	if err != nil {
		t.Fatalf("eval symlinks: %v", err)
	}

	reg := &Registry{Version: 1, Workspaces: []Entry{{Name: "ch1", Path: registered}}}
	_, err = Detect(reg, linkOutside)
	if err == nil {
		t.Fatalf("expected error")
	}

	message := err.Error()
	for _, want := range []string{
		"cwd: " + linkOutside,
		"resolved cwd: " + resolvedOutside,
	} {
		if !strings.Contains(message, want) {
			t.Fatalf("expected error to contain %q, got:\n%s", want, message)
		}
	}
}

func TestDetectErrorIncludesPathContextAndCheckoutHint(t *testing.T) {
	root := t.TempDir()
	workspacePath := filepath.Join(root, "chromium-1", "src")
	if err := os.MkdirAll(filepath.Join(workspacePath, ".git"), 0o755); err != nil {
		t.Fatalf("mkdir workspace: %v", err)
	}
	outsidePath := filepath.Join(root, "outside")
	if err := os.MkdirAll(outsidePath, 0o755); err != nil {
		t.Fatalf("mkdir outside: %v", err)
	}

	reg := &Registry{Version: 1}
	if _, err := reg.Add("ch1", workspacePath); err != nil {
		t.Fatalf("add workspace: %v", err)
	}

	_, err := Detect(reg, outsidePath)
	if err == nil {
		t.Fatalf("expected Detect to fail")
	}
	message := err.Error()
	for _, want := range []string{
		"cwd: " + outsidePath,
		"registered checkouts:",
		"ch1  " + canonicalPath(workspacePath),
		"try: browseros-patch diff ch1",
	} {
		if !strings.Contains(message, want) {
			t.Fatalf("expected error to contain %q, got:\n%s", want, message)
		}
	}
}

func TestRegistryErrorsUseCheckoutTerminology(t *testing.T) {
	file := filepath.Join(t.TempDir(), "not-a-dir")
	if err := os.WriteFile(file, []byte("x"), 0o644); err != nil {
		t.Fatalf("write file: %v", err)
	}

	if _, err := NormalizeWorkspacePath(file); err == nil {
		t.Fatalf("expected not-directory error")
	} else if !strings.Contains(err.Error(), "checkout path is not a directory") {
		t.Fatalf("expected checkout terminology, got %q", err)
	}

	reg := &Registry{Version: 1}
	if _, err := reg.Get("missing"); err == nil {
		t.Fatalf("expected missing checkout error")
	} else if !strings.Contains(err.Error(), `checkout "missing" not found`) {
		t.Fatalf("expected checkout terminology, got %q", err)
	}
}
