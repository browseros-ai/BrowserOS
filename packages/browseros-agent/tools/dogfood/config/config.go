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
	CDP       int `yaml:"cdp"`
	Server    int `yaml:"server"`
	Extension int `yaml:"extension"`
}

type Target string

const (
	TargetBrowserOS Target = "browseros"
	TargetClaw      Target = "claw"
)

type TargetConfig struct {
	DevUserDataDir string `yaml:"dev_user_data_dir"`
	DevProfileDir  string `yaml:"dev_profile_dir"`
	BrowserOSDir   string `yaml:"browseros_dir"`
	Ports          Ports  `yaml:"ports"`
}

type ProductionEnv struct {
	Server map[string]string `yaml:"server"`
	CLI    map[string]string `yaml:"cli"`
}

type Config struct {
	RepoPath          string                  `yaml:"repo_path"`
	BrowserOSAppPath  string                  `yaml:"browseros_app_path"`
	SourceUserDataDir string                  `yaml:"source_user_data_dir"`
	SourceProfileDir  string                  `yaml:"source_profile_dir"`
	DevUserDataDir    string                  `yaml:"dev_user_data_dir"`
	DevProfileDir     string                  `yaml:"dev_profile_dir"`
	BrowserOSDir      string                  `yaml:"browseros_dir"`
	Branch            string                  `yaml:"branch"`
	Ports             Ports                   `yaml:"ports"`
	Target            Target                  `yaml:"-"`
	Targets           map[string]TargetConfig `yaml:"targets"`
	ProductionEnv     ProductionEnv           `yaml:"production_env"`
}

type packageJSON struct {
	Name string `json:"name"`
}

type fileConfig struct {
	RepoPath          string                  `yaml:"repo_path"`
	BrowserOSAppPath  string                  `yaml:"browseros_app_path"`
	SourceUserDataDir string                  `yaml:"source_user_data_dir"`
	SourceProfileDir  string                  `yaml:"source_profile_dir"`
	Branch            string                  `yaml:"branch"`
	Targets           map[string]TargetConfig `yaml:"targets"`
	ProductionEnv     ProductionEnv           `yaml:"production_env"`
}

const (
	LogDirName = "logs"

	DefaultBranch = "main"

	DefaultBrowserOSAppPath   = "/Applications/BrowserOS.app/Contents/MacOS/BrowserOS"
	DefaultBrowserClawAppPath = "/Applications/BrowserClaw.app/Contents/MacOS/BrowserClaw"
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
		ProductionEnv:     DefaultProductionEnv(),
	}
	_ = cfg.ApplyTarget(TargetBrowserOS)
	return cfg
}

// ResolveBrowserAppPath chooses the app binary for a target without freezing discovered defaults into config.
func ResolveBrowserAppPath(target Target, configuredPath string, exists func(string) bool) BrowserAppResolution {
	configuredPath = strings.TrimSpace(configuredPath)
	if configuredPath == "" {
		configuredPath = DefaultBrowserOSAppPath
	}
	if isCustomBrowserAppPath(configuredPath) {
		if target != TargetClaw {
			return BrowserAppResolution{Path: configuredPath, PreferredPath: configuredPath}
		}
		if exists == nil || exists(configuredPath) {
			return BrowserAppResolution{Path: configuredPath, PreferredPath: configuredPath}
		}
		return BrowserAppResolution{
			Path:          fallbackBrowserAppPath(target, exists),
			PreferredPath: configuredPath,
			MissingPath:   configuredPath,
			Fallback:      true,
		}
	}
	if target == TargetClaw {
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
	return BrowserAppResolution{
		Path:          DefaultBrowserOSAppPath,
		PreferredPath: DefaultBrowserOSAppPath,
	}
}

// DefaultTargets returns isolated BrowserOS and BrowserClaw runtime settings.
func DefaultTargets(home string) map[string]TargetConfig {
	cfgDir := DefaultConfigDir(home)
	return map[string]TargetConfig{
		string(TargetBrowserOS): {
			DevUserDataDir: filepath.Join(cfgDir, "browseros", "profile"),
			DevProfileDir:  "Default",
			BrowserOSDir:   filepath.Join(home, ".browseros-dogfood"),
			Ports:          Ports{CDP: 9015, Server: 9115, Extension: 9315},
		},
		string(TargetClaw): {
			DevUserDataDir: filepath.Join(cfgDir, "claw", "profile"),
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
	target := cfg.Target
	cfg.FillProductionEnvDefaults()
	cfg.CaptureTarget()
	cfg.Resolve()
	if target != "" {
		if err := cfg.ApplyTarget(target); err != nil {
			return err
		}
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	data, err := yaml.Marshal(fileConfig{
		RepoPath:          cfg.RepoPath,
		BrowserOSAppPath:  cfg.BrowserOSAppPath,
		SourceUserDataDir: cfg.SourceUserDataDir,
		SourceProfileDir:  cfg.SourceProfileDir,
		Branch:            cfg.Branch,
		Targets:           cfg.Targets,
		ProductionEnv:     cfg.ProductionEnv,
	})
	if err != nil {
		return err
	}
	header := "# browseros-dogfood configuration\n# Run: browseros-dogfood --browseros init or browseros-dogfood --claw init to reconfigure\n\n"
	return os.WriteFile(path, append([]byte(header), data...), 0644)
}

func (c *Config) Resolve() {
	target := c.Target
	if target == "" {
		target = TargetBrowserOS
	}
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
	_ = c.ApplyTarget(target)
	c.FillProductionEnvDefaults()
}

func (c Config) resolveTargets(home string) map[string]TargetConfig {
	defaults := DefaultTargets(home)
	targets := map[string]TargetConfig{}
	for key, value := range defaults {
		targets[key] = value
	}
	for key, value := range c.Targets {
		targets[key] = mergeTargetConfig(value, targets[key], home)
	}
	if len(c.Targets) == 0 {
		legacy := TargetConfig{
			DevUserDataDir: c.DevUserDataDir,
			DevProfileDir:  c.DevProfileDir,
			BrowserOSDir:   c.BrowserOSDir,
			Ports:          c.Ports,
		}
		targets[string(TargetBrowserOS)] = mergeTargetConfig(legacy, targets[string(TargetBrowserOS)], home)
	}
	for key, value := range targets {
		targets[key] = mergeTargetConfig(value, defaultsForTarget(defaults, Target(key)), home)
	}
	return targets
}

func defaultsForTarget(defaults map[string]TargetConfig, target Target) TargetConfig {
	if cfg, ok := defaults[string(target)]; ok {
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
	if value.Ports.Extension != 0 {
		out.Ports.Extension = value.Ports.Extension
	}
	return out
}

// ApplyTarget projects one target's runtime settings onto the active config view.
func (c *Config) ApplyTarget(target Target) error {
	settings, ok := c.Targets[string(target)]
	if !ok {
		return fmt.Errorf("unknown dogfood target %q", target)
	}
	c.Target = target
	c.DevUserDataDir = settings.DevUserDataDir
	c.DevProfileDir = settings.DevProfileDir
	c.BrowserOSDir = settings.BrowserOSDir
	c.Ports = settings.Ports
	return nil
}

// CaptureTarget stores the active runtime settings back under their selected target.
func (c *Config) CaptureTarget() {
	if c.Target == "" {
		return
	}
	if c.Targets == nil {
		c.Targets = map[string]TargetConfig{}
	}
	c.Targets[string(c.Target)] = TargetConfig{
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
	if err := validateRepo(c.AgentRoot()); err != nil {
		return err
	}
	if c.Target != TargetClaw {
		if c.BrowserOSAppPath == "" {
			return fmt.Errorf("browseros_app_path is required")
		}
		if info, err := os.Stat(c.BrowserOSAppPath); err != nil {
			return fmt.Errorf("browseros_app_path: %w", err)
		} else if info.IsDir() || info.Mode()&0111 == 0 {
			return fmt.Errorf("browseros_app_path is not an executable file: %s", c.BrowserOSAppPath)
		}
	}
	return nil
}

func isCustomBrowserAppPath(path string) bool {
	return path != "" && path != DefaultBrowserOSAppPath
}

func fallbackBrowserAppPath(target Target, exists func(string) bool) string {
	if target == TargetClaw && exists != nil && exists(DefaultBrowserClawAppPath) {
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

func DefaultProductionEnv() ProductionEnv {
	return ProductionEnv{
		Server: map[string]string{
			"BROWSEROS_CONFIG_URL": "https://llm.browseros.com/api/browseros-server/config",
			"POSTHOG_API_KEY":      "",
			"SENTRY_DSN":           "",
			"R2_ACCOUNT_ID":        "",
			"R2_ACCESS_KEY_ID":     "",
			"R2_SECRET_ACCESS_KEY": "",
			"R2_BUCKET":            "",
			"R2_DOWNLOAD_PREFIX":   "artifacts/vendor",
			"R2_UPLOAD_PREFIX":     "artifacts/server",
			"NODE_ENV":             "production",
			"LOG_LEVEL":            "debug",
		},
		CLI: map[string]string{
			"POSTHOG_API_KEY":      "",
			"R2_ACCOUNT_ID":        "",
			"R2_ACCESS_KEY_ID":     "",
			"R2_SECRET_ACCESS_KEY": "",
			"R2_BUCKET":            "browseros",
			"R2_UPLOAD_PREFIX":     "",
		},
	}
}

func (c *Config) FillProductionEnvDefaults() {
	defaults := DefaultProductionEnv()
	if c.ProductionEnv.Server == nil {
		c.ProductionEnv.Server = map[string]string{}
	}
	if c.ProductionEnv.CLI == nil {
		c.ProductionEnv.CLI = map[string]string{}
	}
	for key, value := range defaults.Server {
		if _, ok := c.ProductionEnv.Server[key]; !ok {
			c.ProductionEnv.Server[key] = value
		}
	}
	for key, value := range defaults.CLI {
		if _, ok := c.ProductionEnv.CLI[key]; !ok {
			c.ProductionEnv.CLI[key] = value
		}
	}
}
