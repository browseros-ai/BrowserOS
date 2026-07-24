package cmd

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"

	"browseros-dogfood/config"
)

type dogfoodSidecarConfig struct {
	Ports       dogfoodSidecarPorts       `json:"ports"`
	Directories dogfoodSidecarDirectories `json:"directories"`
	Flags       dogfoodSidecarFlags       `json:"flags"`
	Instance    dogfoodSidecarInstance    `json:"instance"`
}

type dogfoodSidecarPorts struct {
	Server int `json:"server"`
	CDP    int `json:"cdp"`
	Proxy  int `json:"proxy"`
}

type dogfoodSidecarDirectories struct {
	Resources string `json:"resources"`
	Execution string `json:"execution"`
}

type dogfoodSidecarFlags struct {
	AllowRemoteInMCP bool `json:"allow_remote_in_mcp"`
}

type dogfoodSidecarInstance struct {
	ClientID         string `json:"client_id"`
	InstallID        string `json:"install_id"`
	BrowserOSVersion string `json:"browseros_version"`
	ChromiumVersion  string `json:"chromium_version"`
}

func dogfoodSidecarConfigPath(cfg config.Config) string {
	return filepath.Join(cfg.DevUserDataDir, "server-config.json")
}

func writeDogfoodSidecarConfig(path string, cfg config.Config, resourcesDir string) error {
	if cfg.Ports.Server == 0 || cfg.Ports.CDP == 0 {
		return fmt.Errorf("dogfood sidecar ports require non-zero server and cdp values")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(dogfoodSidecarConfig{
		Ports: dogfoodSidecarPorts{
			Server: cfg.Ports.Server,
			CDP:    cfg.Ports.CDP,
			Proxy:  cfg.Ports.Server,
		},
		Directories: dogfoodSidecarDirectories{
			Resources: resourcesDir,
			Execution: cfg.BrowserOSDir,
		},
		Flags: dogfoodSidecarFlags{
			AllowRemoteInMCP: false,
		},
	}, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(data, '\n'), 0644)
}
