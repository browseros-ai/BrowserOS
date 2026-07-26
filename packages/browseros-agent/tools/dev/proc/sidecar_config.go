package proc

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
)

type SidecarConfigOptions struct {
	Ports        Ports
	ResourcesDir string
	ExecutionDir string
}

type sidecarConfigFile struct {
	Ports       sidecarPorts       `json:"ports"`
	Directories sidecarDirectories `json:"directories"`
	Flags       sidecarFlags       `json:"flags"`
	Instance    sidecarInstance    `json:"instance"`
}

type sidecarPorts struct {
	Server int `json:"server"`
	CDP    int `json:"cdp"`
	Proxy  int `json:"proxy"`
}

type sidecarDirectories struct {
	Resources string `json:"resources"`
	Execution string `json:"execution"`
}

type sidecarFlags struct {
	AllowRemoteInMCP bool `json:"allow_remote_in_mcp"`
}

type sidecarInstance struct {
	ClientID         string `json:"client_id"`
	InstallID        string `json:"install_id"`
	BrowserOSVersion string `json:"browseros_version"`
	ChromiumVersion  string `json:"chromium_version"`
}

// WriteSidecarConfig writes the sidecar JSON file consumed by local server binaries.
func WriteSidecarConfig(path string, opts SidecarConfigOptions) error {
	if opts.Ports.Server == 0 || opts.Ports.CDP == 0 {
		return fmt.Errorf("sidecar ports require non-zero server and cdp values")
	}
	if opts.ResourcesDir == "" || opts.ExecutionDir == "" {
		return fmt.Errorf("sidecar directories require resources and execution paths")
	}
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(sidecarConfigFile{
		Ports: sidecarPorts{
			Server: opts.Ports.Server,
			CDP:    opts.Ports.CDP,
			Proxy:  opts.Ports.Server,
		},
		Directories: sidecarDirectories{
			Resources: opts.ResourcesDir,
			Execution: opts.ExecutionDir,
		},
		Flags: sidecarFlags{
			AllowRemoteInMCP: false,
		},
	}, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(data, '\n'), 0644)
}
