package cmd

import (
	"os"
	"path/filepath"
	"testing"

	"browseros-dev/proc"
)

func TestResolveDevTargetClearsTheWatchPorts(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, ".env.development"), []byte(
		"BROWSEROS_CDP_PORT=9101\nBROWSEROS_SERVER_PORT=9201\nBROWSEROS_EXTENSION_PORT=9301\n",
	), 0o644); err != nil {
		t.Fatal(err)
	}
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("BROWSEROS_DIR", "")

	target, err := resolveResetTarget(root, resetTargetOptions{Target: "dev"})
	if err != nil {
		t.Fatal(err)
	}

	// claw-server's standalone port replaces the env file's BROWSEROS_SERVER_PORT, as in watch.
	if target.Ports == nil || target.Ports.CDP != 9101 || target.Ports.Server != 9200 || target.Ports.Extension != 9301 {
		t.Fatalf("unexpected ports: %#v", target.Ports)
	}
	if target.BrowserOSDir != filepath.Join(home, ".browseros-dev") {
		t.Fatalf("unexpected browseros dir: %s", target.BrowserOSDir)
	}
	if len(target.BrowserUserDataDirs) != 2 {
		t.Fatalf("unexpected browser user data dirs: %#v", target.BrowserUserDataDirs)
	}
}

func TestResolveDevTargetFallsBackToExampleEnvPorts(t *testing.T) {
	root := t.TempDir()
	if err := os.WriteFile(filepath.Join(root, ".env.development.example"), []byte(
		"BROWSEROS_CDP_PORT=9000\nBROWSEROS_SERVER_PORT=9100\nBROWSEROS_EXTENSION_PORT=9300\n",
	), 0o644); err != nil {
		t.Fatal(err)
	}
	t.Setenv("HOME", t.TempDir())
	t.Setenv("BROWSEROS_DIR", "")

	target, err := resolveResetTarget(root, resetTargetOptions{Target: "dev"})
	if err != nil {
		t.Fatal(err)
	}

	if target.Ports == nil || target.Ports.CDP != 9000 || target.Ports.Server != 9200 || target.Ports.Extension != 9300 {
		t.Fatalf("unexpected ports: %#v", target.Ports)
	}
}

func TestResolveDevTargetPortsFlagWins(t *testing.T) {
	root := writeWatchEnvExample(t, "BROWSEROS_CDP_PORT=9000\nBROWSEROS_SERVER_PORT=9100\nBROWSEROS_EXTENSION_PORT=9300\n")
	t.Setenv("HOME", t.TempDir())
	t.Setenv("BROWSEROS_DIR", "")

	target, err := resolveResetTarget(root, resetTargetOptions{Target: "dev", Ports: "9011,9111,9311"})
	if err != nil {
		t.Fatal(err)
	}

	if target.Ports == nil || *target.Ports != (proc.Ports{CDP: 9011, Server: 9111, Extension: 9311}) {
		t.Fatalf("expected --ports to win, got %#v", target.Ports)
	}
}

func TestResolveDevTargetKeepsWatchOwnershipWithStateOverrides(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	state := filepath.Join(t.TempDir(), "redirected-state")
	t.Setenv("BROWSEROS_DIR", state)
	for _, override := range []string{"", filepath.Join(t.TempDir(), "flag-state")} {
		target, err := resolveResetTarget(t.TempDir(), resetTargetOptions{Target: "dev", BrowserOSDir: override})
		if err != nil {
			t.Fatal(err)
		}
		if target.WatchRunStateDir != filepath.Join(home, ".browseros-dev", "runs") {
			t.Fatalf("state override redirected watch ownership metadata: %s", target.WatchRunStateDir)
		}
	}
}

func TestReadPortsFromEnvFileStripsHashComments(t *testing.T) {
	path := filepath.Join(t.TempDir(), ".env")
	if err := os.WriteFile(path, []byte(
		"BROWSEROS_CDP_PORT=9005#comment\nBROWSEROS_SERVER_PORT=9105 # comment\nBROWSEROS_EXTENSION_PORT=9305\n",
	), 0o644); err != nil {
		t.Fatal(err)
	}

	ports, ok, err := readPortsFromEnvFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !ok {
		t.Fatal("expected ports to be found")
	}
	if ports.CDP != 9005 || ports.Server != 9105 || ports.Extension != 9305 {
		t.Fatalf("unexpected ports: %#v", ports)
	}
}

func TestResolveProdTargetUsesBrowserosStateRoot(t *testing.T) {
	root := t.TempDir()
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("BROWSEROS_DIR", "")

	target, err := resolveResetTarget(root, resetTargetOptions{Target: "prod"})
	if err != nil {
		t.Fatal(err)
	}

	if target.BrowserOSDir != filepath.Join(home, ".browseros") {
		t.Fatalf("unexpected browseros dir: %s", target.BrowserOSDir)
	}
	if target.Ports != nil {
		t.Fatalf("prod target should not clear ports by default: %#v", target.Ports)
	}
}
