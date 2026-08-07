package cmd

import (
	"fmt"
	"net"
	"net/http"
	"net/http/httptest"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"testing"
	"time"

	"browseros-cli/config"
)

func TestProbeRunningServerUsesDiscoveryBeforeConfig(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())
	t.Setenv("BROWSEROS_URL", "")

	discoveredServer := newHealthyServer(t)
	configServer := newHealthyServer(t)

	serverDir := filepath.Join(home, ".browseros")
	if err := os.MkdirAll(serverDir, 0755); err != nil {
		t.Fatalf("os.MkdirAll() error = %v", err)
	}
	data := []byte(fmt.Sprintf(`{"url":%q}`, discoveredServer.URL))
	if err := os.WriteFile(filepath.Join(serverDir, "server.json"), data, 0644); err != nil {
		t.Fatalf("os.WriteFile() error = %v", err)
	}
	if err := config.Save(&config.Config{ServerURL: configServer.URL}); err != nil {
		t.Fatalf("config.Save() error = %v", err)
	}

	got := probeRunningServer()
	if got != normalizeServerURL(discoveredServer.URL) {
		t.Fatalf("probeRunningServer() = %q, want %q", got, normalizeServerURL(discoveredServer.URL))
	}
}

func TestWaitForServerUsesCommonPortFallback(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("USERPROFILE", home)

	server := newHealthyServer(t)
	port := serverPort(t, server.URL)

	originalPorts := commonBrowserOSPorts
	commonBrowserOSPorts = []int{port}
	t.Cleanup(func() {
		commonBrowserOSPorts = originalPorts
	})

	got, ok := waitForServer(100 * time.Millisecond)
	if !ok {
		t.Fatal("waitForServer() ok = false, want true")
	}
	if got != normalizeServerURL(server.URL) {
		t.Fatalf("waitForServer() = %q, want %q", got, normalizeServerURL(server.URL))
	}
}

func newHealthyServer(t *testing.T) *httptest.Server {
	t.Helper()

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/system/health" {
			http.NotFound(w, r)
			return
		}
		w.WriteHeader(http.StatusOK)
	}))
	t.Cleanup(server.Close)
	return server
}

func serverPort(t *testing.T, rawURL string) int {
	t.Helper()

	parsed, err := url.Parse(rawURL)
	if err != nil {
		t.Fatalf("url.Parse() error = %v", err)
	}
	_, portText, err := net.SplitHostPort(parsed.Host)
	if err != nil {
		t.Fatalf("net.SplitHostPort() error = %v", err)
	}
	port, err := strconv.Atoi(portText)
	if err != nil {
		t.Fatalf("strconv.Atoi() error = %v", err)
	}
	return port
}
