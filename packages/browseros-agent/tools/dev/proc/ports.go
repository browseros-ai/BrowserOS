package proc

import (
	"fmt"
	"math/rand"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"time"
)

type Ports struct {
	CDP       int
	Server    int
	Extension int
}

type PortReservations struct {
	CDP       net.Listener
	Server    net.Listener
	Extension net.Listener
}

const (
	randomPortMin = 9000
	randomPortMax = 9999
)

var defaultLocalPorts = Ports{CDP: 9000, Server: 9100, Extension: 9300}

func DefaultLocalPorts() Ports {
	return defaultLocalPorts
}

func ResolveWatchPorts(useRandom bool) (Ports, *PortReservations, error) {
	return ResolveWatchPortsWithDefaults(DefaultLocalPorts(), useRandom)
}

// ResolveWatchPortsWithDefaults reserves watch ports using caller-selected preferred ports.
func ResolveWatchPortsWithDefaults(defaultPorts Ports, useRandom bool) (Ports, *PortReservations, error) {
	reserved := make(map[int]struct{}, 3)
	reservations := &PortReservations{}
	if useRandom {
		rng := rand.New(rand.NewSource(time.Now().UnixNano()))
		cdp, cdpListener, err := selectRandomPort(rng, reserved)
		if err != nil {
			reservations.ReleaseAll()
			return Ports{}, nil, err
		}
		reservations.CDP = cdpListener
		server, serverListener, err := selectRandomPort(rng, reserved)
		if err != nil {
			reservations.ReleaseAll()
			return Ports{}, nil, err
		}
		reservations.Server = serverListener
		extension, extensionListener, err := selectRandomPort(rng, reserved)
		if err != nil {
			reservations.ReleaseAll()
			return Ports{}, nil, err
		}
		reservations.Extension = extensionListener
		return Ports{CDP: cdp, Server: server, Extension: extension}, reservations, nil
	}

	cdp, cdpListener, err := selectPreferredPort(defaultPorts.CDP, reserved)
	if err != nil {
		reservations.ReleaseAll()
		return Ports{}, nil, err
	}
	reservations.CDP = cdpListener
	server, serverListener, err := selectPreferredPort(defaultPorts.Server, reserved)
	if err != nil {
		reservations.ReleaseAll()
		return Ports{}, nil, err
	}
	reservations.Server = serverListener
	extension, extensionListener, err := selectPreferredPort(defaultPorts.Extension, reserved)
	if err != nil {
		reservations.ReleaseAll()
		return Ports{}, nil, err
	}
	reservations.Extension = extensionListener
	return Ports{CDP: cdp, Server: server, Extension: extension}, reservations, nil
}

func IsPortAvailable(port int) bool {
	ln, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		return false
	}
	ln.Close()
	return true
}

func KillPorts(p Ports) {
	KillPort(p.CDP)
	KillPort(p.Server)
	KillPort(p.Extension)
}

func (r *PortReservations) ReleaseCDP() {
	if r == nil || r.CDP == nil {
		return
	}
	r.CDP.Close()
	r.CDP = nil
}

func (r *PortReservations) ReleaseServer() {
	if r == nil || r.Server == nil {
		return
	}
	r.Server.Close()
	r.Server = nil
}

func (r *PortReservations) ReleaseExtension() {
	if r == nil || r.Extension == nil {
		return
	}
	r.Extension.Close()
	r.Extension = nil
}

func (r *PortReservations) ReleaseAll() {
	if r == nil {
		return
	}
	r.ReleaseCDP()
	r.ReleaseServer()
	r.ReleaseExtension()
}

func KillPort(port int) {
	exec.Command("sh", "-c", fmt.Sprintf("lsof -ti:%d | xargs kill -9 2>/dev/null || true", port)).Run()
}

func KillPortAndWait(port int, timeout time.Duration) error {
	deadline := time.Now().Add(timeout)
	for {
		KillPort(port)
		if IsPortAvailable(port) {
			return nil
		}
		if time.Now().After(deadline) {
			return fmt.Errorf("port %d is still in use after kill -9 cleanup", port)
		}
		time.Sleep(100 * time.Millisecond)
	}
}

func KillPortsAndWait(p Ports, timeout time.Duration) error {
	for _, port := range []int{p.CDP, p.Server, p.Extension} {
		if err := KillPortAndWait(port, timeout); err != nil {
			return err
		}
	}
	return nil
}

func BuildEnv(p Ports, nodeEnv string) []string {
	env := os.Environ()
	env = append(env,
		fmt.Sprintf("BROWSEROS_CDP_PORT=%d", p.CDP),
		fmt.Sprintf("BROWSEROS_SERVER_PORT=%d", p.Server),
		fmt.Sprintf("BROWSEROS_EXTENSION_PORT=%d", p.Extension),
		fmt.Sprintf("NODE_ENV=%s", nodeEnv),
	)
	return env
}

func CleanupTempDirs(prefixes ...string) int {
	tmpDir := os.TempDir()
	count := 0
	for _, prefix := range prefixes {
		entries, err := filepath.Glob(filepath.Join(tmpDir, prefix+"*"))
		if err != nil {
			continue
		}
		for _, entry := range entries {
			info, err := os.Stat(entry)
			if err != nil || !info.IsDir() {
				continue
			}
			if err := os.RemoveAll(entry); err == nil {
				count++
			}
		}
	}
	return count
}

func FindMonorepoRoot() (string, error) {
	exe, err := os.Executable()
	if err == nil {
		candidate := filepath.Join(filepath.Dir(exe), "../..")
		if isMonorepoRoot(candidate) {
			return filepath.Abs(candidate)
		}
	}

	cwd, err := os.Getwd()
	if err != nil {
		return "", fmt.Errorf("cannot determine working directory: %w", err)
	}

	dir := cwd
	for {
		if isMonorepoRoot(dir) {
			return dir, nil
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	return "", fmt.Errorf("cannot find monorepo root (no package.json with apps/ found from %s)", cwd)
}

func isMonorepoRoot(dir string) bool {
	info, err := os.Stat(filepath.Join(dir, "package.json"))
	if err != nil || info.IsDir() {
		return false
	}
	_, err = os.Stat(filepath.Join(dir, "apps"))
	return err == nil
}

func selectPreferredPort(preferred int, reserved map[int]struct{}) (int, net.Listener, error) {
	if listener, ok := reservePort(preferred, reserved); ok {
		return preferred, listener, nil
	}

	start := preferred + 1
	if preferred < randomPortMin || preferred > randomPortMax {
		start = randomPortMin
	}

	for port := start; port <= randomPortMax; port++ {
		if listener, ok := reservePort(port, reserved); ok {
			return port, listener, nil
		}
	}
	for port := randomPortMin; port < start; port++ {
		if listener, ok := reservePort(port, reserved); ok {
			return port, listener, nil
		}
	}
	return 0, nil, fmt.Errorf("no available port found in range %d-%d", randomPortMin, randomPortMax)
}

func selectRandomPort(rng *rand.Rand, reserved map[int]struct{}) (int, net.Listener, error) {
	candidates := make([]int, 0, randomPortMax-randomPortMin+1)
	for port := randomPortMin; port <= randomPortMax; port++ {
		candidates = append(candidates, port)
	}
	rng.Shuffle(len(candidates), func(i, j int) {
		candidates[i], candidates[j] = candidates[j], candidates[i]
	})
	for _, port := range candidates {
		if listener, ok := reservePort(port, reserved); ok {
			return port, listener, nil
		}
	}
	return 0, nil, fmt.Errorf("no available port found in range %d-%d", randomPortMin, randomPortMax)
}

func reservePort(port int, reserved map[int]struct{}) (net.Listener, bool) {
	if _, exists := reserved[port]; exists {
		return nil, false
	}
	listener, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		return nil, false
	}
	reserved[port] = struct{}{}
	return listener, true
}
