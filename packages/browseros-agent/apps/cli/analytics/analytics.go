package analytics

import (
	"crypto/rand"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
	"time"

	"browseros-cli/config"

	"github.com/posthog/posthog-go"
)

var (
	posthogAPIKey string // set via ldflags
	posthogHost   = "https://us.i.posthog.com"
)

const eventPrefix = "browseros.cli."

var svc *service

type service struct {
	client    posthog.Client
	distinctID string
}

func Init(version string) {
	if posthogAPIKey == "" {
		return
	}

	distinctID := resolveDistinctID()
	if distinctID == "" {
		return
	}

	client, err := posthog.NewWithConfig(posthogAPIKey, posthog.Config{
		Endpoint:        posthogHost,
		BatchSize:       10,
		ShutdownTimeout: 3 * time.Second,
		DefaultEventProperties: posthog.NewProperties().
			Set("cli_version", version).
			Set("os", runtime.GOOS).
			Set("arch", runtime.GOARCH),
	})
	if err != nil {
		return
	}

	svc = &service{client: client, distinctID: distinctID}
}

func Track(command string, success bool, duration time.Duration) {
	if svc == nil {
		return
	}
	svc.client.Enqueue(posthog.Capture{
		DistinctId: svc.distinctID,
		Event:      eventPrefix + "command_executed",
		Properties: posthog.NewProperties().
			Set("command", command).
			Set("success", success).
			Set("duration_ms", duration.Milliseconds()).
			Set("$process_person_profile", false),
	})
}

func Close() {
	if svc == nil {
		return
	}
	svc.client.Close()
	svc = nil
}

func resolveDistinctID() string {
	if id := loadBrowserosID(); id != "" {
		return id
	}
	return loadOrCreateInstallID()
}

func loadBrowserosID() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}
	data, err := os.ReadFile(filepath.Join(home, ".browseros", "server.json"))
	if err != nil {
		return ""
	}
	var sc struct {
		BrowserosID string `json:"browseros_id"`
	}
	if json.Unmarshal(data, &sc) != nil {
		return ""
	}
	return sc.BrowserosID
}

func loadOrCreateInstallID() string {
	dir := config.Dir()
	idPath := filepath.Join(dir, "install_id")

	data, err := os.ReadFile(idPath)
	if err == nil {
		if id := strings.TrimSpace(string(data)); id != "" {
			return id
		}
	}

	id := generateUUID()
	os.MkdirAll(dir, 0755)
	os.WriteFile(idPath, []byte(id), 0644)
	return id
}

func generateUUID() string {
	var b [16]byte
	rand.Read(b[:])
	b[6] = (b[6] & 0x0f) | 0x40 // version 4
	b[8] = (b[8] & 0x3f) | 0x80 // variant 2
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:])
}
