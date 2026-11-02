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

type Config struct {
	RepoPath          string
	BrowserOSAppPath  string
	SourceUserDataDir string
	SourceProfileDir  string
	DevUserDataDir    string
	DevProfileDir     string
	BrowserOSDir      string
	Branch            string
	Ports             Ports
}

type packageJSON struct {
	Name string `json:"name"`
}

type fileConfig struct {
	RepoPath          string      `yaml:"repo_path"`
	BrowserOSAppPath  string      `yaml:"browseros_app_path"`
	SourceUserDataDir string      `yaml:"source_user_data_dir"`
	SourceProfileDir  string      `yaml:"source_profile_dir"`
	Branch            string      `yaml:"branch"`
	Targets           fileTargets `yaml:"targets"`
}

// fileTargets keeps the runtime settings under targets.claw, where existing dogfood configs store them.
type fileTargets struct {
	Claw fileRuntime `yaml:"claw"`
}

type fileRuntime struct {
	DevUserDataDir string `yaml:"dev_user_data_dir"`
	DevProfileDir  string `yaml:"dev_profile_dir"`
	BrowserOSDir   string `yaml:"browseros_dir"`
	Ports          Ports  `yaml:"ports"`
}

const (
	LogDirName = "logs"

	// RuntimeDirName names the directory beside config.yaml that holds the profile, lock,
	// socket, and daemon logs. It matches the layout existing dogfood installs already use.
	RuntimeDirName = "claw"

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

// Defaults keeps BrowserOS neo dogfood state apart from the normal app state.
func Defaults(home string) Config {
	return Config{
		BrowserOSAppPath:  DefaultBrowserClawAppPath,
		SourceUserDataDir: filepath.Join(home, "Library/Application Support/BrowserOS"),
		SourceProfileDir:  "Default",
		DevUserDataDir:    filepath.Join(DefaultConfigDir(home), RuntimeDirName, "profile"),
		DevProfileDir:     "Default",
		BrowserOSDir:      filepath.Join(home, ".browseros-claw-dogfood"),
		Branch:            DefaultBranch,
		Ports:             Ports{CDP: 49337, Server: 9200},
	}
}

// ResolveBrowserAppPath chooses the app binary without freezing discovered defaults into config.
// Either default path means BrowserOS neo when installed, else BrowserOS; a custom path wins
// while it exists.
func ResolveBrowserAppPath(configuredPath string, exists func(string) bool) BrowserAppResolution {
	configuredPath = strings.TrimSpace(configuredPath)
	if isCustomBrowserAppPath(configuredPath) {
		if exists == nil || exists(configuredPath) {
			return BrowserAppResolution{Path: configuredPath, PreferredPath: configuredPath}
		}
		return BrowserAppResolution{
			Path:          defaultBrowserAppPath(exists),
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

func Load(path string) (Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return Config{}, err
	}
	var file fileConfig
	if err := yaml.Unmarshal(data, &file); err != nil {
		return Config{}, fmt.Errorf("parse config: %w", err)
	}
	cfg := Config{
		RepoPath:          file.RepoPath,
		BrowserOSAppPath:  file.BrowserOSAppPath,
		SourceUserDataDir: file.SourceUserDataDir,
		SourceProfileDir:  file.SourceProfileDir,
		DevUserDataDir:    file.Targets.Claw.DevUserDataDir,
		DevProfileDir:     file.Targets.Claw.DevProfileDir,
		BrowserOSDir:      file.Targets.Claw.BrowserOSDir,
		Branch:            file.Branch,
		Ports:             file.Targets.Claw.Ports,
	}
	cfg.Resolve()
	return cfg, nil
}

func Save(path string, cfg Config) error {
	cfg.Resolve()
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	data, err := yaml.Marshal(fileConfig{
		RepoPath:          cfg.RepoPath,
		BrowserOSAppPath:  cfg.BrowserOSAppPath,
		SourceUserDataDir: cfg.SourceUserDataDir,
		SourceProfileDir:  cfg.SourceProfileDir,
		Branch:            cfg.Branch,
		Targets: fileTargets{Claw: fileRuntime{
			DevUserDataDir: cfg.DevUserDataDir,
			DevProfileDir:  cfg.DevProfileDir,
			BrowserOSDir:   cfg.BrowserOSDir,
			Ports:          cfg.Ports,
		}},
	})
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
	defaults := Defaults(home)
	if strings.TrimSpace(c.DevUserDataDir) == "" {
		c.DevUserDataDir = defaults.DevUserDataDir
	}
	c.DevUserDataDir = ExpandTilde(c.DevUserDataDir, home)
	c.DevProfileDir = strings.TrimSpace(c.DevProfileDir)
	if c.DevProfileDir == "" {
		c.DevProfileDir = defaults.DevProfileDir
	}
	if strings.TrimSpace(c.BrowserOSDir) == "" {
		c.BrowserOSDir = defaults.BrowserOSDir
	}
	c.BrowserOSDir = ExpandTilde(c.BrowserOSDir, home)
	if c.Ports.CDP == 0 {
		c.Ports.CDP = defaults.Ports.CDP
	}
	if c.Ports.Server == 0 {
		c.Ports.Server = defaults.Ports.Server
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
	return validateRepo(c.AgentRoot())
}

func isCustomBrowserAppPath(path string) bool {
	return path != "" && path != DefaultBrowserOSAppPath && path != DefaultBrowserClawAppPath
}

func defaultBrowserAppPath(exists func(string) bool) string {
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
