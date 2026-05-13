package browser

import (
	"strings"
	"testing"

	"browseros-dev/proc"
)

func TestBuildArgsUsesDevDockIcon(t *testing.T) {
	args := BuildArgs(ArgsConfig{
		Root:              "/repo/packages/browseros-agent",
		Ports:             proc.Ports{CDP: 9005, Server: 9105, Extension: 9305},
		UserDataDir:       "/tmp/browseros-dev",
		LoadDevExtensions: true,
	})
	joined := strings.Join(args, "\n")
	for _, want := range []string{
		"--browseros-dock-icon=dev",
		"--browseros-mcp-port=9105",
		"--browseros-server-port=9105",
		"--browseros-proxy-port=9105",
	} {
		if !strings.Contains(joined, want) {
			t.Fatalf("missing %s in\n%s", want, joined)
		}
	}
}
