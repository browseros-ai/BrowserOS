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
	if cfg.DevUserDataDir != filepath.Join(home, ".config/browseros-dogfood/browseros/profile") {
		t.Fatalf("unexpected dev dir: %s", cfg.DevUserDataDir)
	}
	if cfg.BrowserOSDir != filepath.Join(home, ".browseros-dogfood") {
		t.Fatalf("unexpected BrowserOS dir: %s", cfg.BrowserOSDir)
	}
	if cfg.Branch != "main" {
		t.Fatalf("unexpected branch: %s", cfg.Branch)
	}
	if cfg.LogDir() != filepath.Join(home, ".config/browseros-dogfood/browseros/profile/logs") {
		t.Fatalf("unexpected log dir: %s", cfg.LogDir())
	}
	if cfg.DevProfileDir != "Default" {
		t.Fatalf("unexpected dev profile: %s", cfg.DevProfileDir)
	}
	if cfg.Ports.CDP != 9015 || cfg.Ports.Server != 9115 || cfg.Ports.Extension != 9315 {
		t.Fatalf("unexpected ports: %+v", cfg.Ports)
	}
	if cfg.ProductionEnv.Server["BROWSEROS_CONFIG_URL"] == "" {
		t.Fatalf("missing server production env defaults: %#v", cfg.ProductionEnv.Server)
	}
	if cfg.ProductionEnv.Server["LOG_LEVEL"] != "debug" {
		t.Fatalf("server log level got %q want debug", cfg.ProductionEnv.Server["LOG_LEVEL"])
	}
	if cfg.ProductionEnv.CLI["R2_BUCKET"] != "browseros" {
		t.Fatalf("missing cli production env defaults: %#v", cfg.ProductionEnv.CLI)
	}
}

func TestClawDefaultsUseSeparateRuntimePaths(t *testing.T) {
	home := t.TempDir()
	t.Setenv("XDG_CONFIG_HOME", "")
	cfg := Defaults(home)
	if err := cfg.ApplyTarget(TargetClaw); err != nil {
		t.Fatal(err)
	}

	if cfg.DevUserDataDir != filepath.Join(home, ".config/browseros-dogfood/claw/profile") {
		t.Fatalf("unexpected claw profile dir: %s", cfg.DevUserDataDir)
	}
	if cfg.BrowserOSDir != filepath.Join(home, ".browseros-claw-dogfood") {
		t.Fatalf("unexpected claw state dir: %s", cfg.BrowserOSDir)
	}
	if cfg.Ports.CDP != 49337 || cfg.Ports.Server != 9200 || cfg.Ports.Extension != 0 {
		t.Fatalf("unexpected claw ports: %+v", cfg.Ports)
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
		Ports:             Ports{CDP: 9015, Server: 9115, Extension: 9315},
		ProductionEnv: ProductionEnv{
			Server: map[string]string{"NODE_ENV": "production"},
			CLI:    map[string]string{"R2_BUCKET": "browseros"},
		},
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
	if got.Ports.Server != 9115 {
		t.Fatalf("server port mismatch: %d", got.Ports.Server)
	}
	if got.BrowserOSDir != cfg.BrowserOSDir {
		t.Fatalf("BrowserOS dir mismatch: %q", got.BrowserOSDir)
	}
	if got.Branch != cfg.Branch {
		t.Fatalf("branch mismatch: %q", got.Branch)
	}
	if got.ProductionEnv.CLI["R2_BUCKET"] != "browseros" {
		t.Fatalf("production env mismatch: %#v", got.ProductionEnv)
	}
}

func TestLoadLegacyFlatConfigMapsRuntimeToBrowserOSTarget(t *testing.T) {
	dir := t.TempDir()
	path := filepath.Join(dir, "config.yaml")
	data := []byte(`
repo_path: /repo
browseros_app_path: /bin/sh
source_user_data_dir: /source
source_profile_dir: Default
dev_user_data_dir: /legacy-profile
dev_profile_dir: Profile 1
browseros_dir: /legacy-state
branch: main
ports:
  cdp: 1111
  server: 2222
  extension: 3333
`)
	if err := os.WriteFile(path, data, 0644); err != nil {
		t.Fatal(err)
	}

	cfg, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}
	browseros := cfg.Targets[string(TargetBrowserOS)]
	if browseros.DevUserDataDir != "/legacy-profile" || browseros.BrowserOSDir != "/legacy-state" {
		t.Fatalf("legacy runtime not mapped to browseros target: %+v", browseros)
	}
	if browseros.Ports != (Ports{CDP: 1111, Server: 2222, Extension: 3333}) {
		t.Fatalf("legacy ports not mapped: %+v", browseros.Ports)
	}
	claw := cfg.Targets[string(TargetClaw)]
	if claw.DevUserDataDir == browseros.DevUserDataDir || claw.BrowserOSDir == browseros.BrowserOSDir {
		t.Fatalf("claw target should keep separate defaults: browseros=%+v claw=%+v", browseros, claw)
	}
}

func TestSaveSelectedClawDoesNotOverwriteBrowserOSTarget(t *testing.T) {
	home := t.TempDir()
	path := filepath.Join(t.TempDir(), "config.yaml")
	cfg := Defaults(home)
	if err := cfg.ApplyTarget(TargetClaw); err != nil {
		t.Fatal(err)
	}
	cfg.DevUserDataDir = "/custom-claw-profile"
	cfg.BrowserOSDir = "/custom-claw-state"
	cfg.Ports = Ports{CDP: 49338, Server: 9201}

	if err := Save(path, cfg); err != nil {
		t.Fatal(err)
	}
	got, err := Load(path)
	if err != nil {
		t.Fatal(err)
	}

	browseros := got.Targets[string(TargetBrowserOS)]
	if browseros.DevUserDataDir == "/custom-claw-profile" || browseros.BrowserOSDir == "/custom-claw-state" {
		t.Fatalf("browseros target was overwritten by claw runtime: %+v", browseros)
	}
	claw := got.Targets[string(TargetClaw)]
	if claw.DevUserDataDir != "/custom-claw-profile" || claw.BrowserOSDir != "/custom-claw-state" {
		t.Fatalf("claw target was not saved: %+v", claw)
	}
}

func TestResolveDefaultsBranch(t *testing.T) {
	cfg := Config{}

	cfg.Resolve()

	if cfg.Branch != "main" {
		t.Fatalf("branch got %q want main", cfg.Branch)
	}
}

func TestResolvePreservesSelectedTarget(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	cfg := Defaults(home)
	if err := cfg.ApplyTarget(TargetClaw); err != nil {
		t.Fatal(err)
	}

	cfg.Resolve()

	if cfg.Target != TargetClaw {
		t.Fatalf("target got %q want claw", cfg.Target)
	}
	if cfg.DevUserDataDir != filepath.Join(home, ".config/browseros-dogfood/claw/profile") {
		t.Fatalf("dev profile got %q", cfg.DevUserDataDir)
	}
}

func TestResolveBrowserAppPath(t *testing.T) {
	customPath := filepath.Join(t.TempDir(), "CustomBrowser")
	tests := []struct {
		name          string
		target        Target
		configured    string
		existingPaths map[string]bool
		want          BrowserAppResolution
	}{
		{
			name:          "BrowserOS default uses BrowserOS even when BrowserClaw exists",
			target:        TargetBrowserOS,
			configured:    DefaultBrowserOSAppPath,
			existingPaths: map[string]bool{DefaultBrowserClawAppPath: true},
			want: BrowserAppResolution{
				Path:          DefaultBrowserOSAppPath,
				PreferredPath: DefaultBrowserOSAppPath,
			},
		},
		{
			name:          "Claw default uses BrowserClaw when installed",
			target:        TargetClaw,
			configured:    DefaultBrowserOSAppPath,
			existingPaths: map[string]bool{DefaultBrowserClawAppPath: true},
			want: BrowserAppResolution{
				Path:          DefaultBrowserClawAppPath,
				PreferredPath: DefaultBrowserClawAppPath,
			},
		},
		{
			name:   "Claw default falls back to BrowserOS when BrowserClaw is absent",
			target: TargetClaw,
			want: BrowserAppResolution{
				Path:          DefaultBrowserOSAppPath,
				PreferredPath: DefaultBrowserClawAppPath,
				MissingPath:   DefaultBrowserClawAppPath,
				Fallback:      true,
			},
		},
		{
			name:          "custom configured path wins when available",
			target:        TargetClaw,
			configured:    customPath,
			existingPaths: map[string]bool{customPath: true, DefaultBrowserClawAppPath: true},
			want: BrowserAppResolution{
				Path:          customPath,
				PreferredPath: customPath,
			},
		},
		{
			name:          "BrowserOS custom path remains strict",
			target:        TargetBrowserOS,
			configured:    customPath,
			existingPaths: map[string]bool{DefaultBrowserOSAppPath: true},
			want: BrowserAppResolution{
				Path:          customPath,
				PreferredPath: customPath,
			},
		},
		{
			name:          "missing custom path falls back to target default",
			target:        TargetClaw,
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
			got := ResolveBrowserAppPath(tt.target, tt.configured, func(path string) bool {
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
		Ports:             Ports{CDP: 9015, Server: 9115, Extension: 9315},
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
		Ports:             Ports{CDP: 9015, Server: 9115, Extension: 9315},
	}
	if err := cfg.Validate(); err != nil {
		t.Fatalf("validate: %v", err)
	}
}

func TestResolveExpandsBrowserOSDir(t *testing.T) {
	home := t.TempDir()
	t.Setenv("HOME", home)
	cfg := Config{BrowserOSDir: "~/.browseros-dogfood"}

	cfg.Resolve()

	want := filepath.Join(home, ".browseros-dogfood")
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
		Ports:             Ports{CDP: 9015, Server: 9115, Extension: 9315},
	}

	if err := cfg.Validate(); err == nil {
		t.Fatal("expected missing BrowserOS dir to fail validation")
	}
}
