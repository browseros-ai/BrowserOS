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
	if !strings.Contains(joined, "--browseros-dock-icon=dev") {
		t.Fatalf("missing dev dock icon arg in\n%s", joined)
	}
}
