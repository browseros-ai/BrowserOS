package config

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"

	"browseros-dogfood/internal/fspath"

	"gopkg.in/yaml.v3"
)

type Ports struct {
	CDP    int `yaml:"cdp"`
	Server int `yaml:"server"`
}

// ClawTarget names BrowserOS neo's section of the config file (targets.claw) and its run
// directory (<config dir>/claw). Keeping that layout leaves existing dogfood profiles,
// locks and logs where they already are.
const ClawTarget = "claw"

// TargetConfig is one section under targets: the copied profile, the state root and the
// ports a dogfood run uses.
type TargetConfig struct {
	DevUserDataDir string `yaml:"dev_user_data_dir"`
	DevProfileDir  string `yaml:"dev_profile_dir"`
	BrowserOSDir   string `yaml:"browseros_dir"`
	Ports          Ports  `yaml:"ports"`
}

// Config is the dogfood config file plus a flat runtime view of its claw section. The
// runtime fields are never written as top-level keys. Resolve copies them out of
// Targets[ClawTarget], and Save copies them back first.
type Config struct {
	RepoPath          string                  `yaml:"repo_path"`
	BrowserOSAppPath  string                  `yaml:"browseros_app_path"`
	SourceUserDataDir string                  `yaml:"source_user_data_dir"`
	SourceProfileDir  string                  `yaml:"source_profile_dir"`
	Branch            string                  `yaml:"branch"`
	Targets           map[string]TargetConfig `yaml:"targets"`

	DevUserDataDir string `yaml:"-"`
	DevProfileDir  string `yaml:"-"`
	BrowserOSDir   string `yaml:"-"`
	Ports          Ports  `yaml:"-"`
}

type packageJSON struct {
	Name string `json:"name"`
}

const (
	LogDirName = "logs"

	DefaultBranch = "main"

	DefaultBrowserOSAppPath   = "/Applications/BrowserOS.app/Contents/MacOS/BrowserOS"
	DefaultBrowserClawAppPath = "/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo"
)

type BrowserAppResolution struct {
	Path          string
	PreferredPath string
	MissingPath   string
	Fallback      bool
}

func Path() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(DefaultConfigDir(home), "config.yaml"), nil
}

func DefaultConfigDir(home string) string {
	if xdg := os.Getenv("XDG_CONFIG_HOME"); xdg != "" {
		return filepath.Join(xdg, "browseros-dogfood")
	}
	return filepath.Join(home, ".config", "browseros-dogfood")
}

func Defaults(home string) Config {
	cfg := Config{
		BrowserOSAppPath:  DefaultBrowserOSAppPath,
		SourceUserDataDir: filepath.Join(home, "Library/Application Support/BrowserOS"),
		SourceProfileDir:  "Default",
		Branch:            DefaultBranch,
		Targets:           DefaultTargets(home),
	}
	cfg.applyClaw()
	return cfg
}

// ResolveBrowserAppPath chooses the app binary at launch without freezing discovered
// defaults into config. The default prefers BrowserOS neo and falls back to BrowserOS when
// neo is not installed. A missing custom path falls back the same way.
func ResolveBrowserAppPath(configuredPath string, exists func(string) bool) BrowserAppResolution {
	configuredPath = strings.TrimSpace(configuredPath)
	if configuredPath == "" {
		configuredPath = DefaultBrowserOSAppPath
	}
	if isCustomBrowserAppPath(configuredPath) {
		if exists == nil || exists(configuredPath) {
			return BrowserAppResolution{Path: configuredPath, PreferredPath: configuredPath}
		}
		return BrowserAppResolution{
			Path:          fallbackBrowserAppPath(exists),
			PreferredPath: configuredPath,
			MissingPath:   configuredPath,
			Fallback:      true,
		}
	}
	if exists != nil && exists(DefaultBrowserClawAppPath) {
		return BrowserAppResolution{
			Path:          DefaultBrowserClawAppPath,
			PreferredPath: DefaultBrowserClawAppPath,
		}
	}
	return BrowserAppResolution{
		Path:          DefaultBrowserOSAppPath,
		PreferredPath: DefaultBrowserClawAppPath,
		MissingPath:   DefaultBrowserClawAppPath,
		Fallback:      true,
	}
}

// DefaultTargets returns the BrowserOS neo runtime settings, kept apart from the source
// profile and from the installed app's own state.
func DefaultTargets(home string) map[string]TargetConfig {
	cfgDir := DefaultConfigDir(home)
	return map[string]TargetConfig{
		ClawTarget: {
			DevUserDataDir: filepath.Join(cfgDir, ClawTarget, "profile"),
			DevProfileDir:  "Default",
			BrowserOSDir:   filepath.Join(home, ".browseros-claw-dogfood"),
			Ports:          Ports{CDP: 49337, Server: 9200},
		},
	}
}

func Load(path string) (Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Config{}, err
	}
	var cfg Config
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return Config{}, fmt.Errorf("parse config: %w", err)
	}
	cfg.Resolve()
	return cfg, nil
}

func Save(path string, cfg Config) error {
	cfg.captureClaw()
	cfg.Resolve()
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	data, err := yaml.Marshal(cfg)
	if err != nil {
		return err
	}
	header := "# browseros-dogfood configuration\n# Run: browseros-dogfood init to reconfigure\n\n"
	return os.WriteFile(path, append([]byte(header), data...), 0644)
}

func (c *Config) Resolve() {
	home, err := os.UserHomeDir()
	if err != nil {
		home = ""
	}
	c.RepoPath = ExpandTilde(c.RepoPath, home)
	c.SourceUserDataDir = ExpandTilde(c.SourceUserDataDir, home)
	c.BrowserOSAppPath = ExpandTilde(c.BrowserOSAppPath, home)
	c.Branch = strings.TrimSpace(c.Branch)
	if c.Branch == "" {
		c.Branch = DefaultBranch
	}
	c.Targets = c.resolveTargets(home)
	c.applyClaw()
}

// resolveTargets fills the claw section's defaults. It keeps any other section too, minus
// fields TargetConfig does not know, so a save here does not delete the sections another
// dogfood build reads from the same file.
func (c Config) resolveTargets(home string) map[string]TargetConfig {
	defaults := DefaultTargets(home)
	targets := map[string]TargetConfig{}
	for key, value := range defaults {
		targets[key] = value
	}
	for key, value := range c.Targets {
		targets[key] = mergeTargetConfig(value, targets[key], home)
	}
	for key, value := range targets {
		targets[key] = mergeTargetConfig(value, defaultsForTarget(defaults, key), home)
	}
	return targets
}

func defaultsForTarget(defaults map[string]TargetConfig, target string) TargetConfig {
	if cfg, ok := defaults[target]; ok {
		return cfg
	}
	return TargetConfig{DevProfileDir: "Default"}
}

func mergeTargetConfig(value TargetConfig, fallback TargetConfig, home string) TargetConfig {
	out := fallback
	if strings.TrimSpace(value.DevUserDataDir) != "" {
		out.DevUserDataDir = ExpandTilde(value.DevUserDataDir, home)
	}
	if strings.TrimSpace(value.DevProfileDir) != "" {
		out.DevProfileDir = strings.TrimSpace(value.DevProfileDir)
	}
	if strings.TrimSpace(value.BrowserOSDir) != "" {
		out.BrowserOSDir = ExpandTilde(value.BrowserOSDir, home)
	}
	if value.Ports.CDP != 0 {
		out.Ports.CDP = value.Ports.CDP
	}
	if value.Ports.Server != 0 {
		out.Ports.Server = value.Ports.Server
	}
	return out
}

// applyClaw projects the claw section onto the runtime view.
func (c *Config) applyClaw() {
	settings := c.Targets[ClawTarget]
	c.DevUserDataDir = settings.DevUserDataDir
	c.DevProfileDir = settings.DevProfileDir
	c.BrowserOSDir = settings.BrowserOSDir
	c.Ports = settings.Ports
}

// captureClaw stores the runtime view back under the claw section.
func (c *Config) captureClaw() {
	if c.Targets == nil {
		c.Targets = map[string]TargetConfig{}
	}
	c.Targets[ClawTarget] = TargetConfig{
		DevUserDataDir: c.DevUserDataDir,
		DevProfileDir:  c.DevProfileDir,
		BrowserOSDir:   c.BrowserOSDir,
		Ports:          c.Ports,
	}
}

func (c Config) AgentRoot() string {
	return filepath.Join(c.RepoPath, "packages/browseros-agent")
}

func (c Config) SourceProfilePath() string {
	return filepath.Join(c.SourceUserDataDir, c.SourceProfileDir)
}

func (c Config) DevProfilePath() string {
	return filepath.Join(c.DevUserDataDir, c.DevProfileDir)
}

func (c Config) LogDir() string {
	return filepath.Join(c.DevUserDataDir, LogDirName)
}

func (c Config) LogPath(name string) string {
	return filepath.Join(c.LogDir(), name)
}

// Validate checks the saved settings. Start resolves and checks the browser app instead,
// because the default path moves to BrowserOS neo once neo is installed.
func (c Config) Validate() error {
	if c.RepoPath == "" {
		return fmt.Errorf("repo_path is required")
	}
	if c.SourceUserDataDir == "" || c.SourceProfileDir == "" {
		return fmt.Errorf("source_user_data_dir and source_profile_dir are required")
	}
	if c.DevUserDataDir == "" || c.DevProfileDir == "" {
		return fmt.Errorf("dev_user_data_dir and dev_profile_dir are required")
	}
	if c.BrowserOSDir == "" {
		return fmt.Errorf("browseros_dir is required")
	}
	if fspath.IsSameOrChild(c.DevUserDataDir, c.SourceUserDataDir) {
		return fmt.Errorf("dev_user_data_dir must not equal or live inside source_user_data_dir")
	}
	return validateRepo(c.AgentRoot())
}

func isCustomBrowserAppPath(path string) bool {
	return path != "" && path != DefaultBrowserOSAppPath
}

func fallbackBrowserAppPath(exists func(string) bool) string {
	if exists != nil && exists(DefaultBrowserClawAppPath) {
		return DefaultBrowserClawAppPath
	}
	return DefaultBrowserOSAppPath
}

func validateRepo(agentRoot string) error {
	data, err := os.ReadFile(filepath.Join(agentRoot, "package.json"))
	if err != nil {
		return fmt.Errorf("repo_path must contain packages/browseros-agent/package.json: %w", err)
	}
	var pkg packageJSON
	if err := json.Unmarshal(data, &pkg); err != nil {
		return fmt.Errorf("parse package.json: %w", err)
	}
	if pkg.Name != "browseros-monorepo" {
		return fmt.Errorf("unexpected package name %q in packages/browseros-agent/package.json", pkg.Name)
	}
	return nil
}

func ExpandTilde(path string, home string) string {
	if path == "~" {
		return home
	}
	if strings.HasPrefix(path, "~/") {
		return filepath.Join(home, path[2:])
	}
	return path
}
