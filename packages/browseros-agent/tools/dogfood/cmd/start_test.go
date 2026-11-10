package cmd

import (
	"reflect"
	"strings"
	"testing"

	"browseros-dogfood/config"
)

func TestClawCommandsUseStandaloneWXTAndRustServer(t *testing.T) {
	app := clawAppCommand()
	wantApp := []string{"bun", "--env-file=../../.env.development", "wxt"}
	if !reflect.DeepEqual(app, wantApp) {
		t.Fatalf("claw app command got %#v want %#v", app, wantApp)
	}

	server := clawServerCommand("/tmp/claw-server.json")
	wantServer := []string{"cargo", "run", "-p", "claw-server", "--", "--config", "/tmp/claw-server.json"}
	if !reflect.DeepEqual(server, wantServer) {
		t.Fatalf("claw server command got %#v want %#v", server, wantServer)
	}
}

func TestReportProgressInvokesConfiguredProgress(t *testing.T) {
	var got []string
	reportProgress(environmentOptions{
		Progress: func(message string) {
			got = append(got, message)
		},
	}, "checking repo")

	want := []string{"checking repo"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("progress got %#v want %#v", got, want)
	}
}

func TestClawRuntimeEnvWiresBrowserAndAPISettings(t *testing.T) {
	got := clawRuntimeEnv([]string{
		"PATH=/bin",
		"BROWSEROS_DIR=/wrong-browseros",
		"BROWSERCLAW_DIR=/wrong",
		"BROWSEROS_CLAW_CDP_PORT=1",
		"VITE_BROWSEROS_CLAW_API_URL=http://wrong",
	}, config.Config{
		BrowserOSAppPath: config.DefaultBrowserClawAppPath,
		DevUserDataDir:   "/tmp/claw-profile",
		BrowserOSDir:     "/tmp/claw-state",
		Ports:            config.Ports{CDP: 49337, Server: 9200},
	})

	assertEnvContains(t, got, "PATH=/bin")
	assertEnvContains(t, got, "NODE_ENV=development")
	assertEnvContains(t, got, "BROWSERCLAW_DIR=/tmp/claw-state")
	assertEnvMissingPrefix(t, got, "BROWSEROS_DIR=")
	assertEnvContains(t, got, "BROWSEROS_BINARY=/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo")
	assertEnvContains(t, got, "BROWSEROS_USER_DATA_DIR=/tmp/claw-profile")
	assertEnvContains(t, got, "BROWSEROS_CLAW_CDP_PORT=49337")
	assertEnvContains(t, got, "BROWSEROS_SERVER_PORT=9200")
	assertEnvContains(t, got, "VITE_BROWSEROS_CLAW_API_URL=http://127.0.0.1:9200")
	if strings.Contains(strings.Join(got, "\n"), "http://wrong") {
		t.Fatalf("inherited claw API URL was not overridden: %#v", got)
	}
}

func TestFormatPortsLabelsTheServerPortAPI(t *testing.T) {
	got := formatPorts(config.Ports{CDP: 49337, Server: 9200})
	if got != "CDP=49337 API=9200" {
		t.Fatalf("got %q", got)
	}
}

func TestResolveConfiguredBrowserAppUsesClawDefaultAtStart(t *testing.T) {
	cfg := config.Config{BrowserOSAppPath: config.DefaultBrowserOSAppPath}

	resolution := resolveConfiguredBrowserApp(&cfg, func(path string) bool {
		return path == config.DefaultBrowserClawAppPath
	})

	if cfg.BrowserOSAppPath != config.DefaultBrowserClawAppPath {
		t.Fatalf("resolved app path got %q", cfg.BrowserOSAppPath)
	}
	if resolution.Fallback {
		t.Fatalf("did not expect fallback: %#v", resolution)
	}
}

func TestResolveConfiguredBrowserAppFallsBackWhenCustomPathIsMissing(t *testing.T) {
	cfg := config.Config{BrowserOSAppPath: "/missing/custom-browser"}

	resolution := resolveConfiguredBrowserApp(&cfg, func(path string) bool {
		return path == config.DefaultBrowserClawAppPath
	})

	if cfg.BrowserOSAppPath != config.DefaultBrowserClawAppPath {
		t.Fatalf("resolved app path got %q", cfg.BrowserOSAppPath)
	}
	if !resolution.Fallback || resolution.MissingPath != "/missing/custom-browser" {
		t.Fatalf("unexpected fallback resolution: %#v", resolution)
	}
}

func assertEnvContains(t *testing.T, env []string, want string) {
	t.Helper()
	for _, entry := range env {
		if entry == want {
			return
		}
	}
	t.Fatalf("env missing %q: %#v", want, env)
}

func assertEnvMissingPrefix(t *testing.T, env []string, prefix string) {
	t.Helper()
	for _, entry := range env {
		if strings.HasPrefix(entry, prefix) {
			t.Fatalf("env unexpectedly contains %q: %#v", prefix, env)
		}
	}
}
