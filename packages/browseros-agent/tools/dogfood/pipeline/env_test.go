package pipeline

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"browseros-dogfood/config"
)

func TestWriteProductionEnvFile(t *testing.T) {
	root := t.TempDir()
	cfg := config.Config{
		ProductionEnv: config.ProductionEnv{
			Server: map[string]string{
				"NODE_ENV":   "production",
				"LOG_LEVEL":  "debug",
				"R2_BUCKET":  "server-bucket",
				"SHARED_KEY": "server",
			},
			CLI: map[string]string{
				"R2_BUCKET":  "cli-bucket",
				"SHARED_KEY": "cli",
			},
		},
	}
	if err := WriteProductionEnvFile(root, cfg); err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(root, ".env.production")
	assertMode(t, path, 0600)
	assertMissing(t, filepath.Join(root, "apps", "server", ".env.production"))
	assertMissing(t, filepath.Join(root, "apps", "cli", ".env.production"))
	assertContains(t, path, "BROWSEROS_CONFIG_URL=https://llm.browseros.com/api/browseros-server/config\n")
	assertContains(t, path, "LOG_LEVEL=debug\n")
	assertContains(t, path, "NODE_ENV=production\n")
	assertContains(t, path, "POSTHOG_API_KEY=\n")
	assertContains(t, path, "R2_BUCKET=server-bucket\n")
	assertContains(t, path, "SHARED_KEY=server\n")
	assertNotContains(t, path, "R2_UPLOAD_PREFIX")
	assertNotContains(t, path, "R2_DOWNLOAD_PREFIX")
}

func TestWriteProductionEnvFileKeepsCLIDefaultWhenServerDefaultIsEmpty(t *testing.T) {
	root := t.TempDir()
	if err := WriteProductionEnvFile(root, config.Config{}); err != nil {
		t.Fatal(err)
	}

	assertContains(t, filepath.Join(root, ".env.production"), "R2_BUCKET=browseros\n")
}

func TestWriteProductionEnvFileWritesServerOnlyEmptyValues(t *testing.T) {
	root := t.TempDir()
	if err := WriteProductionEnvFile(root, config.Config{}); err != nil {
		t.Fatal(err)
	}

	assertContains(t, filepath.Join(root, ".env.production"), "SENTRY_DSN=\n")
}

func TestWriteEnvFileQuotesUnsafeValues(t *testing.T) {
	path := filepath.Join(t.TempDir(), ".env.production")
	if err := writeEnvFile(path, map[string]string{"TOKEN": "abc=123 with space"}); err != nil {
		t.Fatal(err)
	}
	assertContains(t, path, "TOKEN=\"abc=123 with space\"\n")
}

func TestWriteEnvFileRejectsNewlines(t *testing.T) {
	path := filepath.Join(t.TempDir(), ".env.production")
	if err := writeEnvFile(path, map[string]string{"TOKEN": "abc\n123"}); err == nil {
		t.Fatal("expected newline value error")
	}
}

func assertContains(t *testing.T, path string, want string) {
	t.Helper()
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(got), want) {
		t.Fatalf("%s missing %q in %q", path, want, string(got))
	}
}

func assertNotContains(t *testing.T, path string, want string) {
	t.Helper()
	got, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(got), want) {
		t.Fatalf("%s unexpectedly contains %q in %q", path, want, string(got))
	}
}

func assertMissing(t *testing.T, path string) {
	t.Helper()
	if _, err := os.Stat(path); !os.IsNotExist(err) {
		t.Fatalf("%s should not exist, stat err %v", path, err)
	}
}

func assertMode(t *testing.T, path string, want os.FileMode) {
	t.Helper()
	info, err := os.Stat(path)
	if err != nil {
		t.Fatal(err)
	}
	if got := info.Mode().Perm(); got != want {
		t.Fatalf("%s mode got %o want %o", path, got, want)
	}
}
