package cmd

import (
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"browseros-dev/proc"
)

func TestClawWatchReusesStateAcrossWorktreeProfiles(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("BROWSERCLAW_DIR", "")
	want := filepath.Join(home, ".browserclaw-dev")
	for _, profile := range []string{"/tmp/browseros-dev-first", "/tmp/browseros-dev-second"} {
		env, err := buildWatchEnv(proc.Ports{}, profile, true, false)
		if err != nil {
			t.Fatal(err)
		}
		if got := envValue(env, "BROWSERCLAW_DIR"); got != want {
			t.Fatalf("profile %s selected Claw state %s, want %s", profile, got, want)
		}
		if got := envValue(env, "BROWSEROS_USER_DATA_DIR"); got != profile {
			t.Fatalf("browser profile = %s, want %s", got, profile)
		}
	}
}

func TestNewClawWatchCreatesSeparateTemporaryStateRoots(t *testing.T) {
	tempRoot := t.TempDir()
	t.Setenv("TMPDIR", tempRoot)
	t.Setenv("BROWSERCLAW_DIR", "")
	var previous string
	for range 2 {
		profile, err := os.MkdirTemp(tempRoot, "browseros-dev-")
		if err != nil {
			t.Fatal(err)
		}
		env, err := buildWatchEnv(proc.Ports{}, profile, true, true)
		if err != nil {
			t.Fatal(err)
		}
		state := envValue(env, "BROWSERCLAW_DIR")
		if filepath.Dir(state) != tempRoot || !strings.HasPrefix(filepath.Base(state), "browserclaw-dev-") {
			t.Fatalf("expected an independent temporary Claw root, got %s", state)
		}
		if info, err := os.Stat(state); err != nil || !info.IsDir() {
			t.Fatalf("Claw root was not created: %s (%v)", state, err)
		}
		if state == previous {
			t.Fatalf("new watch reused Claw state %s", state)
		}
		previous = state
	}
}

func TestClawWatchStateOverrideWinsInPlainAndNewModes(t *testing.T) {
	home := t.TempDir()
	tempRoot := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("TMPDIR", tempRoot)
	workingDir := t.TempDir()
	t.Chdir(workingDir)
	for _, tc := range []struct{ override, want string }{
		{filepath.Join(home, "copy-prod"), filepath.Join(home, "copy-prod")},
		{"~/.browserclaw-copy-prod", filepath.Join(home, ".browserclaw-copy-prod")},
		{" ./relative-claw ", filepath.Join(workingDir, "relative-claw")},
	} {
		t.Setenv("BROWSERCLAW_DIR", tc.override)
		for _, fresh := range []bool{false, true} {
			env, err := buildWatchEnv(proc.Ports{}, "/tmp/profile", true, fresh)
			if err != nil {
				t.Fatal(err)
			}
			var assignments []string
			for _, entry := range env {
				if strings.HasPrefix(entry, "BROWSERCLAW_DIR=") {
					assignments = append(assignments, entry)
				}
			}
			if len(assignments) != 1 || assignments[0] != "BROWSERCLAW_DIR="+tc.want {
				t.Fatalf("override %q (fresh=%t): state assignments %q, want %s", tc.override, fresh, assignments, tc.want)
			}
		}
	}
	entries, err := os.ReadDir(tempRoot)
	if err != nil || len(entries) != 0 {
		t.Fatalf("an explicit override should not allocate temporary state: entries=%v, err=%v", entries, err)
	}
}

func TestNewBrowserOSWatchKeepsItsExistingStateRoot(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("BROWSEROS_DIR", "")
	env, err := buildWatchEnv(proc.Ports{}, "/tmp/profile", false, true)
	if err != nil {
		t.Fatal(err)
	}
	if got := envValue(env, "BROWSEROS_DIR"); got != filepath.Join(home, ".browseros-dev") {
		t.Fatalf("unexpected BrowserOS state root: %s", got)
	}
}

func TestClawWatchDirectoryAliasesShareStateOwnership(t *testing.T) {
	parent := t.TempDir()
	alias := filepath.Join(t.TempDir(), "state-alias")
	if err := os.Symlink(parent, alias); err != nil {
		t.Fatal(err)
	}
	// Neither selected root exists yet; ownership must still resolve through
	// the aliased parent instead of creating two locks on the first run.
	first, err := resolveClawWatchStateLockDir(filepath.Join(alias, "new-state", "nested"))
	if err != nil {
		t.Fatal(err)
	}
	second, err := resolveClawWatchStateLockDir(filepath.Join(parent, "new-state", "nested"))
	if err != nil {
		t.Fatal(err)
	}
	if first != second {
		t.Fatalf("one state directory selected two ownership identities: %s, %s", first, second)
	}
}

func TestDevCleanupRemovesTemporaryClawStateAndPreservesSavedState(t *testing.T) {
	home := t.TempDir()
	tempRoot := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("TMPDIR", tempRoot)
	t.Setenv("BROWSERCLAW_DIR", "")
	t.Setenv("BROWSEROS_DIR", "")
	profile, err := os.MkdirTemp(tempRoot, "browseros-dev-")
	if err != nil {
		t.Fatal(err)
	}
	env, err := buildWatchEnv(proc.Ports{}, profile, true, true)
	if err != nil {
		t.Fatal(err)
	}
	savedState := filepath.Join(home, ".browserclaw-dev")
	if err := os.MkdirAll(savedState, 0o755); err != nil {
		t.Fatal(err)
	}
	marker := filepath.Join(savedState, "settings.json")
	if err := os.WriteFile(marker, []byte("saved settings"), 0o600); err != nil {
		t.Fatal(err)
	}
	target, err := resolveResetTarget(t.TempDir(), resetTargetOptions{Target: "dev"})
	if err != nil {
		t.Fatal(err)
	}
	if err := runSafeCleanup(io.Discard, target, safeCleanupOptions{temps: true}); err != nil {
		t.Fatal(err)
	}
	for _, dir := range []string{profile, envValue(env, "BROWSERCLAW_DIR")} {
		if _, err := os.Stat(dir); !os.IsNotExist(err) {
			t.Fatalf("temporary directory survived cleanup: %s (%v)", dir, err)
		}
	}
	if contents, err := os.ReadFile(marker); err != nil || string(contents) != "saved settings" {
		t.Fatalf("saved Claw state was changed by cleanup: %q (%v)", contents, err)
	}
}
