package cmd

import (
	"fmt"
	"net/http"
	"net/http/httptest"
	"testing"

	"browseros-cli/config"
)

func TestInitCommandAcceptsClawMCPURL(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", t.TempDir())

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		switch r.URL.Path {
		case "/health":
			http.NotFound(w, r)
		case "/system/health":
			fmt.Fprint(w, `{"status":"ok"}`)
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(server.Close)

	cmd, _, err := rootCmd.Find([]string{"init"})
	if err != nil {
		t.Fatalf("rootCmd.Find(init) error = %v", err)
	}

	cmd.Run(cmd, []string{server.URL + "/mcp"})

	cfg, err := config.Load()
	if err != nil {
		t.Fatalf("config.Load() error = %v", err)
	}
	if cfg.ServerURL != server.URL {
		t.Fatalf("saved server URL = %q, want %q", cfg.ServerURL, server.URL)
	}
}
