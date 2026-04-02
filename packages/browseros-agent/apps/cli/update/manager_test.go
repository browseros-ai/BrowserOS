package update

import (
	"context"
	"net/http"
	"net/http/httptest"
	"runtime"
	"testing"
	"time"
)

func TestManagerCachedNotice(t *testing.T) {
	manager := NewManager(Options{
		CurrentVersion: "1.0.0",
		Automatic:      true,
	})
	manager.state = &State{LatestVersion: "1.2.0"}

	notice := manager.CachedNotice()
	if notice == "" {
		t.Fatal("CachedNotice() returned empty notice")
	}
}

func TestManagerShouldCheck(t *testing.T) {
	manager := NewManager(Options{
		CurrentVersion: "1.0.0",
		Automatic:      true,
		CheckTTL:       time.Minute,
		Now: func() time.Time {
			return time.Unix(1000, 0).UTC()
		},
	})
	manager.state = &State{LastCheckedAt: time.Unix(0, 0).UTC()}

	if !manager.ShouldCheck() {
		t.Fatal("ShouldCheck() = false, want true")
	}
}

func TestManagerCheckNow(t *testing.T) {
	configRoot := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configRoot)

	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{
			"version":"9.9.9",
			"published_at":"2026-03-27T19:00:00Z",
			"tag":"browseros-cli-v9.9.9",
			"assets":{
				"` + runtimePlatformKey(t) + `":{
					"filename":"browseros-cli_9.9.9_test.tar.gz",
					"url":"https://cdn.example.com/cli/v9.9.9/browseros-cli_9.9.9_test.tar.gz",
					"archive_format":"tar.gz",
					"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
				}
			}
		}`))
	}))
	defer server.Close()

	manager := NewManager(Options{
		CurrentVersion: "1.0.0",
		ManifestURL:    server.URL,
		Automatic:      false,
		HTTPClient:     server.Client(),
		Now: func() time.Time {
			return time.Unix(100, 0).UTC()
		},
	})

	result, err := manager.CheckNow(context.Background())
	if err != nil {
		t.Fatalf("CheckNow() error = %v", err)
	}
	if !result.UpdateAvailable {
		t.Fatal("CheckNow() UpdateAvailable = false, want true")
	}
	if result.LatestPublishedAt != "2026-03-27T19:00:00Z" {
		t.Fatalf(
			"CheckNow() LatestPublishedAt = %q, want %q",
			result.LatestPublishedAt,
			"2026-03-27T19:00:00Z",
		)
	}
	if manager.state.LatestPublishedAt != "2026-03-27T19:00:00Z" {
		t.Fatalf(
			"state LatestPublishedAt = %q, want %q",
			manager.state.LatestPublishedAt,
			"2026-03-27T19:00:00Z",
		)
	}
}

func TestCloneHTTPClientClearsTimeout(t *testing.T) {
	base := &http.Client{Timeout: time.Second}

	cloned := cloneHTTPClient(base)

	if cloned == base {
		t.Fatal("cloneHTTPClient() returned the original client")
	}
	if cloned.Timeout != 0 {
		t.Fatalf("cloneHTTPClient() Timeout = %s, want 0", cloned.Timeout)
	}
	if base.Timeout != time.Second {
		t.Fatalf("base Timeout = %s, want %s", base.Timeout, time.Second)
	}
}

func TestManagerSaveAppliedState(t *testing.T) {
	configRoot := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", configRoot)

	now := time.Unix(200, 0).UTC()
	manager := NewManager(Options{
		CurrentVersion: "1.0.0",
		Now: func() time.Time {
			return now
		},
	})
	manager.state = &State{
		LastCheckedAt: time.Unix(100, 0).UTC(),
		CheckError:    "manifest fetch failed",
	}

	manager.saveAppliedState(&CheckResult{
		LatestVersion:     "9.9.9",
		LatestPublishedAt: "2026-03-27T19:00:00Z",
		Asset: &Asset{
			URL: "https://cdn.example.com/cli/v9.9.9/browseros-cli_9.9.9_test.tar.gz",
		},
	})

	if manager.state.LastCheckedAt != now {
		t.Fatalf("LastCheckedAt = %v, want %v", manager.state.LastCheckedAt, now)
	}
	if manager.state.CheckError != "" {
		t.Fatalf("CheckError = %q, want empty", manager.state.CheckError)
	}
	if manager.state.LatestPublishedAt != "2026-03-27T19:00:00Z" {
		t.Fatalf("LatestPublishedAt = %q", manager.state.LatestPublishedAt)
	}
}

func TestAutomaticEnabledSkipsForPackageManagerInstall(t *testing.T) {
	t.Setenv("BROWSEROS_INSTALL_METHOD", "npm")

	manager := NewManager(Options{
		CurrentVersion: "1.0.0",
		Automatic:      true,
	})

	if manager.AutomaticEnabled() {
		t.Fatal("AutomaticEnabled() = true, want false when BROWSEROS_INSTALL_METHOD=npm")
	}
}

func TestAutomaticEnabledAllowsNormalInstall(t *testing.T) {
	t.Setenv("BROWSEROS_INSTALL_METHOD", "")

	manager := NewManager(Options{
		CurrentVersion: "1.0.0",
		Automatic:      true,
	})

	if !manager.AutomaticEnabled() {
		t.Fatal("AutomaticEnabled() = false, want true when BROWSEROS_INSTALL_METHOD is empty")
	}
}

func runtimePlatformKey(t *testing.T) string {
	t.Helper()
	key, err := PlatformKey(runtimeGOOS(), runtimeGOARCH())
	if err != nil {
		t.Fatalf("PlatformKey() error = %v", err)
	}
	return key
}

func runtimeGOOS() string {
	return runtime.GOOS
}

func runtimeGOARCH() string {
	return runtime.GOARCH
}
