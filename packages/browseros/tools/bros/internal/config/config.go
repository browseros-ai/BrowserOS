package config

import (
	"fmt"
	"os"
	"path/filepath"

	"gopkg.in/yaml.v3"
)

type Config struct {
	Name        string `yaml:"name"`
	PatchesRepo string `yaml:"patches_repo"`
}

func ReadConfig(brosDir string) (*Config, error) {
	data, err := os.ReadFile(filepath.Join(brosDir, "config.yaml"))
	if err != nil {
		return nil, fmt.Errorf("reading config: %w", err)
	}
	var cfg Config
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parsing config.yaml: %w", err)
	}
	return &cfg, nil
}

func WriteConfig(brosDir string, cfg *Config) error {
	if err := os.MkdirAll(brosDir, 0o755); err != nil {
		return fmt.Errorf("creating .bros directory: %w", err)
	}
	data, err := yaml.Marshal(cfg)
	if err != nil {
		return fmt.Errorf("marshaling config: %w", err)
	}
	path := filepath.Join(brosDir, "config.yaml")
	if err := os.WriteFile(path, data, 0o644); err != nil {
		return fmt.Errorf("writing config.yaml: %w", err)
	}
	return nil
}
