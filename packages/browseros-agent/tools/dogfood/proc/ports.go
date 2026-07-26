package proc

import (
	"fmt"
	"net"
	"os/exec"
	"strings"

	"browseros-dogfood/config"
)

func ResolvePorts(start config.Ports) (config.Ports, bool, error) {
	return ResolveTargetPorts(start, true)
}

func ResolveTargetPorts(start config.Ports, includeExtension bool) (config.Ports, bool, error) {
	used := map[int]bool{}
	cdp, err := resolvePort("CDP", start.CDP, used)
	if err != nil {
		return config.Ports{}, false, err
	}
	used[cdp] = true
	server, err := resolvePort("server", start.Server, used)
	if err != nil {
		return config.Ports{}, false, err
	}
	extension := start.Extension
	if includeExtension {
		used[server] = true
		resolvedExtension, err := resolvePort("extension", start.Extension, used)
		if err != nil {
			return config.Ports{}, false, err
		}
		extension = resolvedExtension
	}
	resolved := config.Ports{CDP: cdp, Server: server, Extension: extension}
	return resolved, resolved != start, nil
}

func resolvePort(name string, start int, used map[int]bool) (int, error) {
	if start <= 0 || start > 65535 {
		return 0, fmt.Errorf("invalid %s port: %d", name, start)
	}
	for port := start; port <= 65535; port++ {
		if used[port] {
			continue
		}
		if isPortAvailable(port) {
			return port, nil
		}
	}
	return 0, fmt.Errorf("no available %s port at or above %d%s", name, start, pidSuffix(start))
}

func isPortAvailable(port int) bool {
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		return false
	}
	ln.Close()
	return true
}

func pidSuffix(port int) string {
	out, err := exec.Command("lsof", "-ti", fmt.Sprintf(":%d", port)).Output()
	if err != nil {
		return ""
	}
	pids := strings.TrimSpace(string(out))
	if pids == "" {
		return ""
	}
	return fmt.Sprintf(" (pids: %s)", strings.ReplaceAll(pids, "\n", ","))
}
