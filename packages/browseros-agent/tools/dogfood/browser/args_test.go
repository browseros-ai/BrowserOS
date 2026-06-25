package browser

import (
	"strings"
	"testing"

	"browseros-dogfood/config"
)

func TestBuildArgs(t *testing.T) {
	args := BuildArgs(ArgsConfig{
		Binary:      "/Applications/BrowserOS.app/Contents/MacOS/BrowserOS",
		AgentRoot:   "/repo/packages/browseros-agent",
		UserDataDir: "/tmp/browseros-dogfood",
		ProfileDir:  "Default",
		Ports:       config.Ports{CDP: 9015, Server: 9115, Extension: 9315},
	})
	joined := strings.Join(args, "\n")
	for _, want := range []string{
		"--remote-debugging-port=9015",
		"--browseros-mcp-port=9115",
		"--browseros-server-port=9115",
		"--browseros-proxy-port=9115",
		"--browseros-extension-port=9315",
		"--user-data-dir=/tmp/browseros-dogfood",
		"--profile-directory=Default",
		"--disable-browseros-server",
		"--disable-browseros-extensions",
		"--browseros-dock-icon=alpha",
		"--enable-logging=stderr",
		"--load-extension=/repo/packages/browseros-agent/apps/app/dist/chrome-mv3-dev",
		"chrome://newtab",
	} {
		if !strings.Contains(joined, want) {
			t.Fatalf("missing %s in\n%s", want, joined)
		}
	}
	if strings.Contains(joined, "--use-mock-keychain") {
		t.Fatal("must not use mock keychain")
	}
}

func TestBuildArgsHeadless(t *testing.T) {
	args := BuildArgs(ArgsConfig{
		Binary:      "/bin/browser",
		AgentRoot:   "/repo/packages/browseros-agent",
		UserDataDir: "/tmp/browseros-dogfood",
		Ports:       config.Ports{CDP: 1, Server: 2, Extension: 3},
		Headless:    true,
	})
	if !contains(args, "--headless=new") {
		t.Fatalf("missing headless arg: %#v", args)
	}
}

func contains(values []string, target string) bool {
	for _, value := range values {
		if value == target {
			return true
		}
	}
	return false
}
