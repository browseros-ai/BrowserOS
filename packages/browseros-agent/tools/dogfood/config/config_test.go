package config

import (
	"os"
	"path/filepath"
	"testing"
)

func TestDefaults(t *testing.T) {
	home := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", "")
	cfg := Defaults(home)

	if cfg.BrowserOSAppPath != DefaultBrowserOSAppPath {
		t.Fatalf("unexpected browser path: %s", cfg.BrowserOSAppPath)
	}
	if cfg.SourceUserDataDir != filepath.Join(home, "Library/Application Support/BrowserOS") {
		t.Fatalf("unexpected source dir: %s", cfg.SourceUserDataDir)
	}
	if cfg.DevUserDataDir != filepath.Join(home, ".config/browseros-dogfood/claw/profile") {
		t.Fatalf("unexpected dev dir: %s", cfg.DevUserDataDir)
	}
	if cfg.BrowserOSDir != filepath.Join(home, ".browseros-claw-dogfood") {
		t.Fatalf("unexpected claw state dir: %s", cfg.BrowserOSDir)
	}
	if cfg.Branch != "main" {
		t.Fatalf("unexpected branch: %s", cfg.Branch)
	}
	if cfg.LogDir() != filepath.Join(home, ".config/browseros-dogfood/claw/profile/logs") {
		t.Fatalf("unexpected log dir: %s", cfg.LogDir())
	}
	if cfg.DevProfileDir != "Default" {
		t.Fatalf("unexpected dev profile: %s", cfg.DevProfileDir)
	}
	if cfg.Ports != (Ports{CDP: 49337, Server: 9200}) {
		t.Fatalf("unexpected ports: %+v", cfg.Ports)
	}
}

func TestLogPathUsesProfileLogDir(t *testing.T) {
	cfg := Config{DevUserDataDir: "/tmp/browseros-dogfood-profile"}
	got := cfg.LogPath("server.log")
	want := filepath.Join("/tmp/browseros-dogfood-profile", "logs", "server.log")
	if got != want {
		t.Fatalf("got %q want %q", got, want)
	}
}

func TestSaveLoadRoundTrip(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yaml")
	cfg := Config{
		RepoPath:          "/repo",
		BrowserOSAppPath:  DefaultBrowserOSAppPath,
		SourceUserDataDir: "/source",
		SourceProfileDir:  "Profile 25",
		DevUserDataDir:    "/dev",
		DevProfileDir:     "Default",
		BrowserOSDir:      "/browseros-dogfood",
		Branch:            "dogfood",
		Ports:             Ports{CDP: 49338, Server: 9201},
	}

	if err := Save(path, cfg); err != nil {
		t.Fatalf("save: %v", err)
	}
	got, err := Load(path)
	if err != nil {
		t.Fatalf("load: %v", err)
	}
	if got.SourceProfileDir != cfg.SourceProfileDir {
		t.Fatalf("source profile mismatch: %q", got.SourceProfileDir)
	}
	if got.Ports != cfg.Ports {
		t.Fatalf("ports mismatch: %+v", got.Ports)
	}
	if got.DevUserDataDir != cfg.DevUserDataDir {
		t.Fatalf("dev profile mismatch: %q", got.DevUserDataDir)
	}
	if got.BrowserOSDir != cfg.BrowserOSDir {
		t.Fatalf("BrowserOS dir mismatch: %q", got.BrowserOSDir)
	}
	if got.Branch != cfg.Branch {
		t.Fatalf("branch mismatch: %q", got.Branch)
	}
}

func TestLoadIgnoresTopLevelRuntimeSettings(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	t.Setenv("XDG_CONFIG_HOME", "")
	path := filepath.Join(t.TempDir(), "config.yaml")
	data := []byte(`
repo_path: /repo
source_user_data_dir: /source
source_profile_dir: Default
dev_user_data_dir: /legacy-profile
browseros_dir: /legacy-state
ports:
  cdp: 1111
  server: 2222
`)
	if err := os.WriteFile(path, data, 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}

	if cfg.DevUserDataDir != filepath.Join(home, ".config/browseros-dogfood/claw/profile") {
		t.Fatalf("top-level profile leaked into the claw runtime: %q", cfg.DevUserDataDir)
	}
	if cfg.BrowserOSDir != filepath.Join(home, ".browseros-claw-dogfood") {
		t.Fatalf("top-level state dir leaked into the claw runtime: %q", cfg.BrowserOSDir)
	}
	if cfg.Ports != (Ports{CDP: 49337, Server: 9200}) {
		t.Fatalf("top-level ports leaked into the claw runtime: %+v", cfg.Ports)
	}
}

func TestSaveKeepsOtherTargetSections(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	path := filepath.Join(t.TempDir(), "config.yaml")
	data := []byte(`
repo_path: /repo
targets:
  browseros:
    dev_user_data_dir: /other-profile
    browseros_dir: /other-state
`)
	if err := os.WriteFile(path, data, 0644); err != nil {
		t.Fatal(err)
	}
	cfg, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	cfg.Ports = Ports{CDP: 49338, Server: 9201}

	if err := Save(path, cfg); err != nil {
		t.Fatal(err)
	}
	got, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}

	other := got.Targets["browseros"]
	if other.DevUserDataDir != "/other-profile" || other.BrowserOSDir != "/other-state" {
		t.Fatalf("other target section was not kept: %+v", other)
	}
	if got.Ports != (Ports{CDP: 49338, Server: 9201}) {
		t.Fatalf("claw ports were not saved: %+v", got.Ports)
	}
}

func TestResolveDefaultsBranch(t *testing.T) {
	cfg := Config{}

	cfg.Resolve()

	if cfg.Branch != "main" {
		t.Fatalf("branch got %q want main", cfg.Branch)
	}
}

func TestResolveBrowserAppPath(t *testing.T) {
	customPath := filepath.Join(t.TempDir(), "CustomBrowser")
	tests := []struct {
		name          string
		configured    string
		existingPaths map[string]bool
		want          BrowserAppResolution
	}{
		{
			name:          "default uses BrowserOS neo when installed",
			configured:    DefaultBrowserOSAppPath,
			existingPaths: map[string]bool{DefaultBrowserClawAppPath: true},
			want: BrowserAppResolution{
				Path:          DefaultBrowserClawAppPath,
				PreferredPath: DefaultBrowserClawAppPath,
			},
		},
		{
			name: "default falls back to BrowserOS when BrowserOS neo is absent",
			want: BrowserAppResolution{
				Path:          DefaultBrowserOSAppPath,
				PreferredPath: DefaultBrowserClawAppPath,
				MissingPath:   DefaultBrowserClawAppPath,
				Fallback:      true,
			},
		},
		{
			name:          "custom configured path wins when available",
			configured:    customPath,
			existingPaths: map[string]bool{customPath: true, DefaultBrowserClawAppPath: true},
			want: BrowserAppResolution{
				Path:          customPath,
				PreferredPath: customPath,
			},
		},
		{
			name:          "missing custom path falls back to BrowserOS neo",
			configured:    customPath,
			existingPaths: map[string]bool{DefaultBrowserClawAppPath: true},
			want: BrowserAppResolution{
				Path:          DefaultBrowserClawAppPath,
				PreferredPath: customPath,
				MissingPath:   customPath,
				Fallback:      true,
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := ResolveBrowserAppPath(tt.configured, func(path string) bool {
				return tt.existingPaths[path]
			})
			if got != tt.want {
				t.Fatalf("ResolveBrowserAppPath got %#v want %#v", got, tt.want)
			}
		})
	}
}

func TestExpandTilde(t *testing.T) {
	got := ExpandTilde("~/x", "/Users/test")
	want := filepath.Join("/Users/test", "x")
	if got != want {
		t.Fatalf("got %q want %q", got, want)
	}
}

func TestValidateRejectsSourceInsideDev(t *testing.T) {
	cfg := Config{
		RepoPath:          t.TempDir(),
		BrowserOSAppPath:  "/bin/sh",
		SourceUserDataDir: "/tmp/source",
		SourceProfileDir:  "Default",
		DevUserDataDir:    "/tmp/source/dev",
		DevProfileDir:     "Default",
		BrowserOSDir:      "/tmp/browseros-dogfood",
		Ports:             Ports{CDP: 49337, Server: 9200},
	}
	if err := cfg.Validate(); err == nil {
		t.Fatal("expected validation error")
	}
}

func TestConfigPathHonorsXDG(t *testing.T) {
	dir := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", dir)
	got, err := Path()
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(dir, "browseros-dogfood", "config.yaml")
	if got != want {
		t.Fatalf("got %q want %q", got, want)
	}
}

func TestPathDefault(t *testing.T) {
	t.Setenv("XDG_CONFIG_HOME", "")
	home := t.TempDir()
	t.Setenv("HOME", home)
	got, err := Path()
	if err != nil {
		t.Fatal(err)
	}
	want := filepath.Join(home, ".config", "browseros-dogfood", "config.yaml")
	if got != want {
		t.Fatalf("got %q want %q", got, want)
	}
}

func TestValidateRepoShape(t *testing.T) {
	repo := t.TempDir()
	agentRoot := filepath.Join(repo, "packages/browseros-agent")
	if err := os.MkdirAll(agentRoot, 0755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(agentRoot, "package.json"), []byte(`{"name":"browseros-monorepo"}`), 0644); err != nil {
		t.Fatal(err)
	}
	cfg := Config{
		RepoPath:          repo,
		BrowserOSAppPath:  "/bin/sh",
		SourceUserDataDir: "/tmp/source",
		SourceProfileDir:  "Default",
		DevUserDataDir:    "/tmp/dev",
		DevProfileDir:     "Default",
		BrowserOSDir:      "/tmp/browseros-dogfood",
		Ports:             Ports{CDP: 49337, Server: 9200},
	}
	if err := cfg.Validate(); err != nil {
		t.Fatalf("validate: %v", err)
	}
}

func TestResolveExpandsBrowserOSDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	cfg := Config{Targets: map[string]TargetConfig{ClawTarget: {BrowserOSDir: "~/.browseros-claw-copy"}}}

	cfg.Resolve()

	want := filepath.Join(home, ".browseros-claw-copy")
	if cfg.BrowserOSDir != want {
		t.Fatalf("expanded BrowserOS dir got %q want %q", cfg.BrowserOSDir, want)
	}
}

func TestValidateRequiresBrowserOSDir(t *testing.T) {
	cfg := Config{
		RepoPath:          t.TempDir(),
		BrowserOSAppPath:  "/bin/sh",
		SourceUserDataDir: "/tmp/source",
		SourceProfileDir:  "Default",
		DevUserDataDir:    "/tmp/dev",
		DevProfileDir:     "Default",
		Ports:             Ports{CDP: 49337, Server: 9200},
	}

	if err := cfg.Validate(); err == nil {
		t.Fatal("expected missing BrowserOS dir to fail validation")
	}
}
