package proc

import (
	"fmt"
	"math/rand"
	"net"
	"testing"
)

func TestSelectPreferredPortUsesPreferredWhenAvailable(t *testing.T) {
	reserved := map[int]struct{}{}
	preferred := findFreePortInRange(t, randomPortMin)

	port, listener, err := selectPreferredPort(preferred, reserved)
	if err != nil {
		t.Fatalf("selectPreferredPort returned error: %v", err)
	}
	defer listener.Close()
	if port != preferred {
		t.Fatalf("expected preferred port %d, got %d", preferred, port)
	}
	if _, ok := reserved[port]; !ok {
		t.Fatalf("expected port %d to be reserved", port)
	}
}

func TestSelectPreferredPortFallsBackWhenPreferredUnavailable(t *testing.T) {
	preferred := findFreePortInRange(t, randomPortMin)
	listener := listenOnPort(t, preferred)
	defer listener.Close()

	reserved := map[int]struct{}{}
	port, reservedListener, err := selectPreferredPort(preferred, reserved)
	if err != nil {
		t.Fatalf("selectPreferredPort returned error: %v", err)
	}
	defer reservedListener.Close()
	if port == preferred {
		t.Fatalf("expected fallback port when preferred port %d is unavailable", preferred)
	}
	if port < randomPortMin || port > randomPortMax {
		t.Fatalf("expected fallback port in range %d-%d, got %d", randomPortMin, randomPortMax, port)
	}
	if _, ok := reserved[port]; !ok {
		t.Fatalf("expected fallback port %d to be reserved", port)
	}
}

func TestSelectRandomPortUsesRangeAndUniqueness(t *testing.T) {
	rng := rand.New(rand.NewSource(1))
	reserved := map[int]struct{}{}

	first, firstListener, err := selectRandomPort(rng, reserved)
	if err != nil {
		t.Fatalf("selectRandomPort returned error: %v", err)
	}
	defer firstListener.Close()
	second, secondListener, err := selectRandomPort(rng, reserved)
	if err != nil {
		t.Fatalf("selectRandomPort returned error: %v", err)
	}
	defer secondListener.Close()
	third, thirdListener, err := selectRandomPort(rng, reserved)
	if err != nil {
		t.Fatalf("selectRandomPort returned error: %v", err)
	}
	defer thirdListener.Close()

	assertPortInRange(t, first)
	assertPortInRange(t, second)
	assertPortInRange(t, third)

	if first == second || first == third || second == third {
		t.Fatalf("expected unique ports, got %d, %d, %d", first, second, third)
	}
}

func TestResolveWatchPortsRandomUsesUniquePortsInRange(t *testing.T) {
	ports, reservations, err := ResolveWatchPorts(true)
	if err != nil {
		t.Fatalf("ResolveWatchPorts returned error: %v", err)
	}
	defer reservations.ReleaseAll()

	assertPortInRange(t, ports.CDP)
	assertPortInRange(t, ports.Server)
	assertPortInRange(t, ports.Extension)

	if ports.CDP == ports.Server || ports.CDP == ports.Extension || ports.Server == ports.Extension {
		t.Fatalf("expected unique ports, got %+v", ports)
	}
	if IsPortAvailable(ports.CDP) || IsPortAvailable(ports.Server) || IsPortAvailable(ports.Extension) {
		t.Fatalf("expected reserved ports to stay bound until release, got %+v", ports)
	}
}

func TestResolveWatchPortsDefaultFallsBackWhenPreferredUnavailable(t *testing.T) {
	defaults := findUniqueFreePorts(t, 3)
	cdpListener := listenOnPort(t, defaults.CDP)
	defer cdpListener.Close()
	serverListener := listenOnPort(t, defaults.Server)
	defer serverListener.Close()
	extensionListener := listenOnPort(t, defaults.Extension)
	defer extensionListener.Close()

	ports, reservations, err := ResolveWatchPortsWithDefaults(defaults, false)
	if err != nil {
		t.Fatalf("ResolveWatchPorts returned error: %v", err)
	}
	defer reservations.ReleaseAll()

	if ports == defaults {
		t.Fatalf("expected fallback ports when defaults are occupied, got %+v", ports)
	}
	if ports.CDP == ports.Server || ports.CDP == ports.Extension || ports.Server == ports.Extension {
		t.Fatalf("expected unique fallback ports, got %+v", ports)
	}
	if IsPortAvailable(ports.CDP) || IsPortAvailable(ports.Server) || IsPortAvailable(ports.Extension) {
		t.Fatalf("expected fallback ports to stay bound until release, got %+v", ports)
	}
}

func assertPortInRange(t *testing.T, port int) {
	t.Helper()
	if port < randomPortMin || port > randomPortMax {
		t.Fatalf("expected port in range %d-%d, got %d", randomPortMin, randomPortMax, port)
	}
}

func findFreePortInRange(t *testing.T, start int) int {
	t.Helper()
	for port := start; port <= randomPortMax; port++ {
		if IsPortAvailable(port) {
			return port
		}
	}
	t.Fatalf("failed to find free port in range %d-%d", start, randomPortMax)
	return 0
}

func findUniqueFreePorts(t *testing.T, count int) Ports {
	t.Helper()
	found := Ports{}
	values := make([]int, 0, count)
	for port := randomPortMin; port <= randomPortMax && len(values) < count; port++ {
		if IsPortAvailable(port) {
			values = append(values, port)
		}
	}
	if len(values) != count {
		t.Fatalf("failed to find %d unique free ports", count)
	}
	found.CDP = values[0]
	found.Server = values[1]
	found.Extension = values[2]
	return found
}

func listenOnPort(t *testing.T, port int) net.Listener {
	t.Helper()
	listener, err := net.Listen("tcp", fmt.Sprintf("127.0.0.1:%d", port))
	if err != nil {
		t.Fatalf("failed to listen on port %d: %v", port, err)
	}
	return listener
}
