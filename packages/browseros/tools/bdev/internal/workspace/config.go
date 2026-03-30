package workspace

import (
	"fmt"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

type Config struct {
	Version     int    `yaml:"version" json:"version"`
	PatchesRepo string `yaml:"patches_repo,omitempty" json:"patches_repo,omitempty"`
}

func ConfigDir() string {
	if xdg := os.Getenv("XDG_CONFIG_HOME"); xdg != "" {
		return filepath.Join(xdg, "bdev")
	}
	home, _ := os.UserHomeDir()
	return filepath.Join(home, ".config", "bdev")
}

func ConfigPath() string {
	return filepath.Join(ConfigDir(), "config.yaml")
}

func RegistryPath() string {
	return filepath.Join(ConfigDir(), "workspaces.yaml")
}

func LoadConfig() (*Config, error) {
	data, err := os.ReadFile(ConfigPath())
	if err != nil {
		if os.IsNotExist(err) {
			return &Config{Version: 1}, nil
		}
		return nil, err
	}
	var cfg Config
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parse config: %w", err)
	}
	if cfg.Version == 0 {
		cfg.Version = 1
	}
	return &cfg, nil
}

func SaveConfig(cfg *Config) error {
	if cfg.Version == 0 {
		cfg.Version = 1
	}
	if err := os.MkdirAll(ConfigDir(), 0o755); err != nil {
		return err
	}
	body, err := yaml.Marshal(cfg)
	if err != nil {
		return err
	}
	header := "# bdev configuration\n\n"
	return os.WriteFile(ConfigPath(), append([]byte(header), body...), 0o644)
}
