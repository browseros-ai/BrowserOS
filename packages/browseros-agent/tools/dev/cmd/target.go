package cmd

import (
	"bufio"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"browseros-dev/proc"
)

const (
	targetDev  = "dev"
	targetProd = "prod"

	devDirName  = ".browseros-dev"
	prodDirName = ".browseros"
)

type resetTargetOptions struct {
	Target       string
	BrowserOSDir string
	Ports        string
}

// resetTarget is the machine state that cleanup and reset act on for one --target. Safe
// cleanup stops its watch owners, browsers and ports and removes its temp directories.
// Reset can also delete the Lima VM under LimaHome and then the BrowserOSDir state root.
type resetTarget struct {
	Name                string
	Title               string
	BrowserOSDir        string
	LimaHome            string
	Ports               *proc.Ports
	BrowserUserDataDirs []string
	TempPrefixes        []string
	WatchRunStateDir    string
	DeleteRootLabel     string
	DeleteRootBody      string
}

func resolveResetTarget(root string, opts resetTargetOptions) (resetTarget, error) {
	target := strings.TrimSpace(opts.Target)
	if target == "" {
		target = targetDev
	}
	switch target {
	case targetDev:
		return resolveDevTarget(root, opts)
	case targetProd:
		return resolveProdTarget(opts)
	default:
		return resetTarget{}, fmt.Errorf("unsupported reset target %q; use dev or prod", target)
	}
}

func resolveDevTarget(root string, opts resetTargetOptions) (resetTarget, error) {
	browserosDir, err := resolveBrowserOSDir(opts.BrowserOSDir, devDirName)
	if err != nil {
		return resetTarget{}, err
	}
	ports, err := resolveTargetPorts(root, opts.Ports)
	if err != nil {
		return resetTarget{}, err
	}
	devProfile, err := proc.DefaultDevUserDataDir(root)
	if err != nil {
		return resetTarget{}, err
	}
	// Watch ownership metadata has one shared home location even when the
	// BrowserOS product state is redirected through BROWSEROS_DIR or a flag.
	watchRunStateDir, err := proc.DefaultWatchRunBaseDir()
	if err != nil {
		return resetTarget{}, err
	}
	return resetTarget{
		Name:                targetDev,
		Title:               "BrowserOS dev reset",
		BrowserOSDir:        browserosDir,
		LimaHome:            filepath.Join(browserosDir, "lima"),
		Ports:               &ports,
		BrowserUserDataDirs: []string{"/tmp/browseros-dev", devProfile},
		TempPrefixes:        []string{"browseros-dev-", clawWatchTempStatePrefix},
		WatchRunStateDir:    watchRunStateDir,
		DeleteRootLabel:     "Delete dev profile?",
		DeleteRootBody:      "It removes BrowserOS dev data plus VM state.",
	}, nil
}

func resolveProdTarget(opts resetTargetOptions) (resetTarget, error) {
	browserosDir, err := resolveBrowserOSDir(opts.BrowserOSDir, prodDirName)
	if err != nil {
		return resetTarget{}, err
	}
	return resetTarget{
		Name:            targetProd,
		Title:           "BrowserOS prod reset",
		BrowserOSDir:    browserosDir,
		LimaHome:        filepath.Join(browserosDir, "lima"),
		DeleteRootLabel: "Delete prod BrowserOS state?",
		DeleteRootBody:  "It removes ~/.browseros server data plus VM state. It does not delete your BrowserOS browser profile.",
	}, nil
}

// resolveBrowserOSDir finds a target's state root. A debug claw-server writes its tool output
// under BROWSEROS_DIR, else ~/.browseros-dev (release builds use ~/.browseros), so the dev root
// follows the same override.
func resolveBrowserOSDir(override string, dirName string) (string, error) {
	if strings.TrimSpace(override) != "" {
		return filepath.Abs(expandTilde(strings.TrimSpace(override)))
	}
	if dirName == devDirName {
		if env := strings.TrimSpace(os.Getenv("BROWSEROS_DIR")); env != "" {
			return filepath.Abs(expandTilde(env))
		}
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, dirName), nil
}

// resolveTargetPorts returns explicit cdp,server,extension ports when given, else the ports
// a plain watch prefers, so dev cleanup clears the ports watch binds.
func resolveTargetPorts(root string, explicit string) (proc.Ports, error) {
	if strings.TrimSpace(explicit) != "" {
		return parsePorts(explicit)
	}
	return resolveWatchDefaultPorts(root)
}

// readEnvFilePorts reads the three BROWSEROS_*_PORT keys from .env.development, then from
// the example file, and falls back to the built-in defaults.
func readEnvFilePorts(root string) (proc.Ports, error) {
	for _, path := range []string{
		filepath.Join(root, ".env.development"),
		filepath.Join(root, ".env.development.example"),
	} {
		ports, ok, err := readPortsFromEnvFile(path)
		if err != nil {
			return proc.Ports{}, err
		}
		if ok {
			return ports, nil
		}
	}
	return proc.DefaultLocalPorts(), nil
}

func readPortsFromEnvFile(path string) (proc.Ports, bool, error) {
	file, err := os.Open(path)
	if os.IsNotExist(err) {
		return proc.Ports{}, false, nil
	}
	if err != nil {
		return proc.Ports{}, false, err
	}
	defer file.Close()

	values := map[string]int{}
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		key, value, ok := parseEnvLine(scanner.Text())
		if !ok {
			continue
		}
		switch key {
		case "BROWSEROS_CDP_PORT", "BROWSEROS_SERVER_PORT", "BROWSEROS_EXTENSION_PORT":
			port, err := strconv.Atoi(value)
			if err != nil {
				return proc.Ports{}, false, fmt.Errorf("parse %s in %s: %w", key, path, err)
			}
			values[key] = port
		}
	}
	if err := scanner.Err(); err != nil {
		return proc.Ports{}, false, err
	}
	if len(values) != 3 {
		return proc.Ports{}, false, nil
	}
	return proc.Ports{
		CDP:       values["BROWSEROS_CDP_PORT"],
		Server:    values["BROWSEROS_SERVER_PORT"],
		Extension: values["BROWSEROS_EXTENSION_PORT"],
	}, true, nil
}

func parseEnvLine(line string) (string, string, bool) {
	line = strings.TrimSpace(line)
	if line == "" || strings.HasPrefix(line, "#") {
		return "", "", false
	}
	key, value, ok := strings.Cut(line, "=")
	if !ok {
		return "", "", false
	}
	key = strings.TrimSpace(key)
	value = strings.TrimSpace(stripInlineComment(value))
	value = strings.Trim(value, `"'`)
	return key, value, key != "" && value != ""
}

func stripInlineComment(value string) string {
	quote := byte(0)
	for index := 0; index < len(value); index++ {
		switch value[index] {
		case '\'', '"':
			if quote == 0 {
				quote = value[index]
			} else if quote == value[index] {
				quote = 0
			}
		case '#':
			if quote == 0 {
				return value[:index]
			}
		}
	}
	return value
}

func parsePorts(value string) (proc.Ports, error) {
	parts := strings.Split(value, ",")
	if len(parts) != 3 {
		return proc.Ports{}, fmt.Errorf("ports must be cdp,server,extension")
	}
	parsed := [3]int{}
	for i, part := range parts {
		port, err := strconv.Atoi(strings.TrimSpace(part))
		if err != nil {
			return proc.Ports{}, fmt.Errorf("parse port %q: %w", part, err)
		}
		if port <= 0 || port > 65535 {
			return proc.Ports{}, fmt.Errorf("port %d out of range", port)
		}
		parsed[i] = port
	}
	return proc.Ports{CDP: parsed[0], Server: parsed[1], Extension: parsed[2]}, nil
}

func expandTilde(path string) string {
	if path == "~" {
		if home, err := os.UserHomeDir(); err == nil {
			return home
		}
	}
	if strings.HasPrefix(path, "~/") {
		if home, err := os.UserHomeDir(); err == nil {
			return filepath.Join(home, path[2:])
		}
	}
	return path
}
