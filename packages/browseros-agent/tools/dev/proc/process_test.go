package proc

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"
	"testing"
	"time"
)

const watchLockHelperEnv = "BROWSEROS_DEV_WATCH_LOCK_HELPER"

func TestMain(m *testing.M) {
	if os.Getenv(watchLockHelperEnv) == "1" {
		runWatchLockHelper()
		return
	}
	os.Exit(m.Run())
}

func TestWatchRunPathsStableAndSharedAcrossPortChanges(t *testing.T) {
	baseDir := t.TempDir()
	identity := WatchRunIdentity{
		Mode:    "watch",
		Profile: "/tmp/browseros-dev",
		Ports:   Ports{CDP: 9005, Server: 9105, Extension: 9305},
	}

	first := watchRunPaths(baseDir, identity)
	second := watchRunPaths(baseDir, identity)
	if first != second {
		t.Fatalf("expected stable paths, got %#v and %#v", first, second)
	}

	withDifferentPort := identity
	withDifferentPort.Ports.Server = 9106
	third := watchRunPaths(baseDir, withDifferentPort)
	if third != first {
		t.Fatalf("expected same paths for different ports, got %#v and %#v", first, third)
	}

	withDifferentProfile := identity
	withDifferentProfile.Profile = "/tmp/browseros-dev-other"
	fourth := watchRunPaths(baseDir, withDifferentProfile)
	if fourth.Lock == first.Lock || fourth.State == first.State {
		t.Fatalf("expected distinct paths for different profiles, got %#v and %#v", first, fourth)
	}
}

func TestBrowserProfilePIDsFromPSSelectsOnlyDevAndTestProfiles(t *testing.T) {
	output := `
  111  111 /Applications/BrowserOS.app/Contents/MacOS/BrowserOS --user-data-dir=/tmp/browseros-dev
  222  222 /Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo --user-data-dir=/tmp/browseros-dev-abcd
  333  333 /Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo --user-data-dir=/var/folders/x/browseros-test-abcd
  444  444 /Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo --user-data-dir=/Users/me/Library/Application Support/BrowserOS
  555  555 rg browseros-test-
`

	pids := browserProfilePIDsFromPS(output)

	if len(pids) != 3 || pids[0] != 111 || pids[1] != 222 || pids[2] != 333 {
		t.Fatalf("expected dev/test browser pids, got %#v", pids)
	}
}

func TestDefaultDevUserDataDirIsWorktreeScoped(t *testing.T) {
	first := filepath.Join(t.TempDir(), "main-2", "packages", "browseros-agent")
	second := filepath.Join(t.TempDir(), "feat-new-mcp", "packages", "browseros-agent")

	firstProfile, err := DefaultDevUserDataDir(first)
	if err != nil {
		t.Fatal(err)
	}
	secondProfile, err := DefaultDevUserDataDir(second)
	if err != nil {
		t.Fatal(err)
	}

	if firstProfile == secondProfile {
		t.Fatalf("expected distinct profiles, got %s", firstProfile)
	}
	if !strings.Contains(firstProfile, "browseros-dev-main-2-") {
		t.Fatalf("expected worktree label in %s", firstProfile)
	}
}

func TestAcquireWatchRunLockWritesStateAndReleases(t *testing.T) {
	baseDir := t.TempDir()
	identity := WatchRunIdentity{
		Mode:    "watch",
		Profile: "/tmp/browseros-dev",
		Ports:   Ports{CDP: 9005, Server: 9105, Extension: 9305},
	}

	lock, stopped, err := AcquireWatchRunLockInDir(baseDir, identity, time.Second)
	if err != nil {
		t.Fatalf("AcquireWatchRunLockInDir returned error: %v", err)
	}
	if stopped {
		t.Fatal("expected first acquisition not to stop another run")
	}

	paths := watchRunPaths(baseDir, identity)
	state, err := ReadWatchRunState(paths.State)
	if err != nil {
		t.Fatalf("ReadWatchRunState returned error: %v", err)
	}
	if state.PID != os.Getpid() {
		t.Fatalf("expected state PID %d, got %d", os.Getpid(), state.PID)
	}
	if state.PGID <= 0 {
		t.Fatalf("expected positive PGID, got %d", state.PGID)
	}
	if state.Identity != identity {
		t.Fatalf("expected identity %#v, got %#v", identity, state.Identity)
	}
	if err := lock.Close(); err != nil {
		t.Fatalf("closing lock: %v", err)
	}
	if _, err := os.Stat(paths.State); !os.IsNotExist(err) {
		t.Fatalf("expected state file to be removed on close, got %v", err)
	}
	if _, err := os.Stat(paths.Lock); err != nil {
		t.Fatalf("expected lock file path to remain reusable, got %v", err)
	}

	lock, stopped, err = AcquireWatchRunLockInDir(baseDir, identity, time.Second)
	if err != nil {
		t.Fatalf("reacquiring lock returned error: %v", err)
	}
	if stopped {
		t.Fatal("expected reacquisition after close not to stop another run")
	}
	if err := lock.Close(); err != nil {
		t.Fatalf("closing reacquired lock: %v", err)
	}
}

func TestAcquireWatchRunLockRejectsInvalidPorts(t *testing.T) {
	identity := WatchRunIdentity{
		Mode:    "watch",
		Profile: "/tmp/browseros-dev",
		Ports:   Ports{CDP: 9005, Server: 65536, Extension: 9305},
	}

	if _, _, err := AcquireWatchRunLockInDir(t.TempDir(), identity, time.Second); err == nil {
		t.Fatal("expected invalid port error")
	}
}

func TestAcquireWatchRunLockStopsExistingOwnerByStatePGID(t *testing.T) {
	baseDir := t.TempDir()
	readyPath := filepath.Join(baseDir, "ready")
	identity := WatchRunIdentity{
		Mode:    "watch",
		Profile: "/tmp/browseros-dev",
		Ports:   Ports{CDP: 9005, Server: 9105, Extension: 9305},
	}
	identityJSON, err := json.Marshal(identity)
	if err != nil {
		t.Fatal(err)
	}

	cmd := exec.Command(os.Args[0], "-test.run=TestMain")
	cmd.Env = append(os.Environ(),
		watchLockHelperEnv+"=1",
		"BROWSEROS_DEV_WATCH_LOCK_BASE="+baseDir,
		"BROWSEROS_DEV_WATCH_LOCK_READY="+readyPath,
		"BROWSEROS_DEV_WATCH_LOCK_IDENTITY="+string(identityJSON),
	)
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := cmd.Start(); err != nil {
		t.Fatalf("starting helper: %v", err)
	}
	defer cmd.Process.Kill()

	waitForFile(t, readyPath, 3*time.Second)

	lock, stopped, err := AcquireWatchRunLockInDir(baseDir, identity, 3*time.Second)
	if err != nil {
		t.Fatalf("AcquireWatchRunLockInDir returned error: %v", err)
	}
	defer lock.Close()
	if !stopped {
		t.Fatal("expected takeover to stop existing owner")
	}

	done := make(chan error, 1)
	go func() {
		done <- cmd.Wait()
	}()
	select {
	case <-done:
	case <-time.After(3 * time.Second):
		t.Fatal("expected helper process to exit after takeover")
	}
}

func TestAcquireWatchRunLockStopsExistingOwnerWithDifferentPorts(t *testing.T) {
	baseDir := t.TempDir()
	readyPath := filepath.Join(baseDir, "ready")
	ownerIdentity := WatchRunIdentity{
		Mode:    "watch",
		Profile: "/tmp/browseros-dev",
		Ports:   Ports{CDP: 9005, Server: 9105, Extension: 9305},
	}
	takeoverIdentity := ownerIdentity
	takeoverIdentity.Ports.Server = 9200
	identityJSON, err := json.Marshal(ownerIdentity)
	if err != nil {
		t.Fatal(err)
	}

	cmd := exec.Command(os.Args[0], "-test.run=TestMain")
	cmd.Env = append(os.Environ(),
		watchLockHelperEnv+"=1",
		"BROWSEROS_DEV_WATCH_LOCK_BASE="+baseDir,
		"BROWSEROS_DEV_WATCH_LOCK_READY="+readyPath,
		"BROWSEROS_DEV_WATCH_LOCK_IDENTITY="+string(identityJSON),
	)
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := cmd.Start(); err != nil {
		t.Fatalf("starting helper: %v", err)
	}
	defer cmd.Process.Kill()

	waitForFile(t, readyPath, 3*time.Second)

	lock, stopped, err := AcquireWatchRunLockInDir(baseDir, takeoverIdentity, 3*time.Second)
	if err != nil {
		t.Fatalf("AcquireWatchRunLockInDir returned error: %v", err)
	}
	defer lock.Close()
	if !stopped {
		t.Fatal("expected takeover to stop existing owner")
	}

	done := make(chan error, 1)
	go func() {
		done <- cmd.Wait()
	}()
	select {
	case <-done:
	case <-time.After(3 * time.Second):
		t.Fatal("expected helper process to exit after takeover")
	}
}

func TestAcquireWatchRunLockStopsLegacyPortKeyedOwner(t *testing.T) {
	baseDir := t.TempDir()
	legacyIdentity := WatchRunIdentity{
		Mode:    "watch",
		Profile: "/tmp/browseros-dev",
		Ports:   Ports{CDP: 9005, Server: 9105, Extension: 9305},
	}
	takeoverIdentity := legacyIdentity
	takeoverIdentity.Ports.Server = 9200

	helperBaseDir := t.TempDir()
	legacyPaths := legacyPortKeyedWatchRunPaths(baseDir, legacyIdentity)
	identityJSON, err := json.Marshal(legacyIdentity)
	if err != nil {
		t.Fatal(err)
	}

	readyPath := filepath.Join(baseDir, "legacy-ready")
	cmd := exec.Command(os.Args[0], "-test.run=TestMain")
	cmd.Env = append(os.Environ(),
		watchLockHelperEnv+"=1",
		"BROWSEROS_DEV_WATCH_LOCK_BASE="+helperBaseDir,
		"BROWSEROS_DEV_WATCH_LOCK_READY="+readyPath,
		"BROWSEROS_DEV_WATCH_LOCK_IDENTITY="+string(identityJSON),
	)
	cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
	if err := cmd.Start(); err != nil {
		t.Fatalf("starting legacy owner: %v", err)
	}
	defer cmd.Process.Kill()

	waitForFile(t, readyPath, 3*time.Second)
	helperState, err := ReadWatchRunState(watchRunPaths(helperBaseDir, legacyIdentity).State)
	if err != nil {
		t.Fatalf("reading helper state: %v", err)
	}
	legacyState := helperState
	legacyState.Identity = legacyIdentity
	data, err := json.MarshalIndent(legacyState, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(legacyPaths.State, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}

	// A shell reaps its old watch while a replacement waits for that owner to
	// exit. Reap this helper concurrently too, so kill(pid, 0) cannot see a zombie.
	done := make(chan error, 1)
	go func() {
		done <- cmd.Wait()
	}()
	lock, stopped, err := AcquireWatchRunLockInDir(baseDir, takeoverIdentity, 3*time.Second)
	if err != nil {
		t.Fatalf("AcquireWatchRunLockInDir returned error: %v", err)
	}
	defer lock.Close()
	if !stopped {
		t.Fatal("expected takeover to stop legacy owner")
	}

	select {
	case <-done:
	case <-time.After(3 * time.Second):
		t.Fatal("expected legacy owner process to exit after takeover")
	}
}

func TestAcquireWatchRunLockWaitsForSlowSupervisorCleanup(t *testing.T) {
	for _, scenario := range []string{"takeover", "legacy-takeover", "cleanup"} {
		t.Run(scenario, func(t *testing.T) {
			legacy := scenario == "legacy-takeover"
			baseDir := t.TempDir()
			helperBaseDir := baseDir
			if legacy {
				helperBaseDir = t.TempDir()
			}
			readyPath := filepath.Join(baseDir, "ready")
			releasePath := filepath.Join(baseDir, "cleanup-complete")
			identity := WatchRunIdentity{
				Mode:    "watch-claw-state",
				Profile: "/tmp/claw-shared-state",
				Ports:   Ports{CDP: 9005, Server: 9105, Extension: 9305},
			}
			identityJSON, err := json.Marshal(identity)
			if err != nil {
				t.Fatal(err)
			}
			cmd := exec.Command(os.Args[0], "-test.run=TestMain")
			cmd.Env = append(os.Environ(),
				watchLockHelperEnv+"=1",
				"BROWSEROS_DEV_WATCH_LOCK_BASE="+helperBaseDir,
				"BROWSEROS_DEV_WATCH_LOCK_READY="+readyPath,
				"BROWSEROS_DEV_WATCH_LOCK_IDENTITY="+string(identityJSON),
				"BROWSEROS_DEV_WATCH_LOCK_RELEASE="+releasePath,
			)
			cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}
			if err := cmd.Start(); err != nil {
				t.Fatal(err)
			}
			t.Cleanup(func() {
				_ = cmd.Process.Kill()
				_ = cmd.Wait()
			})
			waitForFile(t, readyPath, 3*time.Second)
			if legacy {
				state, err := os.ReadFile(watchRunPaths(helperBaseDir, identity).State)
				if err != nil {
					t.Fatal(err)
				}
				if err := os.WriteFile(legacyPortKeyedWatchRunPaths(baseDir, identity).State, state, 0o644); err != nil {
					t.Fatal(err)
				}
			}
			var lock *WatchRunLock
			if scenario == "cleanup" {
				_, err = StopAllWatchProcessesInDir(baseDir, 100*time.Millisecond)
			} else {
				lock, _, err = AcquireWatchRunLockInDir(baseDir, identity, 100*time.Millisecond)
			}
			if lock != nil {
				_ = lock.Close()
			}
			if err == nil || !strings.Contains(err.Error(), "still shutting down") {
				t.Fatalf("expected takeover to wait for cleanup, got %v", err)
			}
			if !processLive(cmd.Process.Pid) {
				t.Fatal("takeover killed the supervisor before it finished child cleanup")
			}
			if err := os.WriteFile(releasePath, nil, 0o644); err != nil {
				t.Fatal(err)
			}
			if err := cmd.Wait(); err != nil {
				t.Fatalf("supervisor did not finish its cleanup: %v", err)
			}
			lock, _, err = AcquireWatchRunLockInDir(baseDir, identity, time.Second)
			if err != nil {
				t.Fatalf("takeover after cleanup failed: %v", err)
			}
			_ = lock.Close()
		})
	}
}

func runWatchLockHelper() {
	baseDir := os.Getenv("BROWSEROS_DEV_WATCH_LOCK_BASE")
	readyPath := os.Getenv("BROWSEROS_DEV_WATCH_LOCK_READY")
	var identity WatchRunIdentity
	if err := json.Unmarshal([]byte(os.Getenv("BROWSEROS_DEV_WATCH_LOCK_IDENTITY")), &identity); err != nil {
		os.Exit(2)
	}

	lock, _, err := AcquireWatchRunLockInDir(baseDir, identity, time.Second)
	if err != nil {
		os.Exit(3)
	}
	defer lock.Close()
	releasePath := os.Getenv("BROWSEROS_DEV_WATCH_LOCK_RELEASE")
	var stop chan os.Signal
	if releasePath != "" {
		stop = make(chan os.Signal, 1)
		signal.Notify(stop, syscall.SIGTERM)
		defer signal.Stop(stop)
	}
	if err := os.WriteFile(readyPath, []byte("ready\n"), 0o644); err != nil {
		os.Exit(4)
	}
	if releasePath != "" {
		<-stop
		// Model a supervisor draining a separate server process group. The
		// test, rather than a delay, decides when releasing ownership is safe.
		for {
			if _, err := os.Stat(releasePath); err == nil {
				return
			}
			time.Sleep(10 * time.Millisecond)
		}
	}
	time.Sleep(30 * time.Second)
}

func legacyPortKeyedWatchRunPaths(baseDir string, identity WatchRunIdentity) watchRunPathsResult {
	identity = normalizeWatchRunIdentity(identity)
	sum := sha256.Sum256([]byte(fmt.Sprintf("%s\x00%s\x00%d\x00%d\x00%d",
		identity.Mode,
		identity.Profile,
		identity.Ports.CDP,
		identity.Ports.Server,
		identity.Ports.Extension,
	)))
	key := hex.EncodeToString(sum[:])
	return watchRunPathsResult{
		Lock:  filepath.Join(baseDir, "watch-"+key+".lock"),
		State: filepath.Join(baseDir, "watch-"+key+".json"),
	}
}

func waitForFile(t *testing.T, path string, timeout time.Duration) {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for {
		if _, err := os.Stat(path); err == nil {
			return
		}
		if time.Now().After(deadline) {
			t.Fatalf("timed out waiting for %s", path)
		}
		time.Sleep(50 * time.Millisecond)
	}
}
