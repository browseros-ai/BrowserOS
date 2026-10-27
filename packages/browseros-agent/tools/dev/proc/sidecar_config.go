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
	// BrowserBinary is the browser the server will drive, when the launcher knows it. Its
	// release bundle stamp becomes instance.browseros_version; see NativeBrowserOSVersion.
	BrowserBinary string
	// DevMode marks a dev claw-server launch (flags.devMode). The server then leaves the
	// installed BrowserOS neo's agent configs alone instead of provisioning its own URL.
	DevMode bool
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

// sidecarFlags mixes key styles because each field belongs to a different server: the
// BrowserOS server reads allow_remote_in_mcp, claw-server reads camelCase devMode.
type sidecarFlags struct {
	AllowRemoteInMCP bool `json:"allow_remote_in_mcp"`
	DevMode          bool `json:"devMode,omitempty"`
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
			DevMode:          opts.DevMode,
		},
		Instance: sidecarInstance{
			BrowserOSVersion: NativeBrowserOSVersion(opts.BrowserBinary),
		},
	}, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, append(data, '\n'), 0644)
}
