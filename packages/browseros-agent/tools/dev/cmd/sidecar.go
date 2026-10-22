package cmd

import (
	"path/filepath"

	"browseros-dev/proc"
)

// writeServerSidecarConfig writes a server's launch config. browserBinary, when known, lets
// the config carry the native BrowserOS version the way a browser-launched config does.
func writeServerSidecarConfig(path string, resourcesDir string, executionDir string, p proc.Ports, browserBinary string) error {
	return proc.WriteSidecarConfig(path, proc.SidecarConfigOptions{
		Ports:         p,
		ResourcesDir:  resourcesDir,
		ExecutionDir:  executionDir,
		BrowserBinary: browserBinary,
	})
}

func watchSidecarConfigPath(userDataDir string, name string) string {
	return filepath.Join(userDataDir, "sidecars", name+".json")
}
