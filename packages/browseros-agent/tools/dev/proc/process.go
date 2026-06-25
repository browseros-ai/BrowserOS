package proc

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
	"syscall"
	"time"
)

var errWatchRunLocked = errors.New("dev watch run is already locked")

const maxTCPPort = 65535

type WatchRunIdentity struct {
	Mode    string `json:"mode"`
	Profile string `json:"profile"`
	Ports   Ports  `json:"ports"`
}

type WatchRunState struct {
	PID       int              `json:"pid"`
	PGID      int              `json:"pgid"`
	StartedAt time.Time        `json:"started_at"`
	Identity  WatchRunIdentity `json:"identity"`
}

type WatchRunLock struct {
	file      *os.File
	statePath string
}

type watchRunPathsResult struct {
	Lock  string
	State string
}

// AcquireWatchRunLock claims ownership of the current dev watch identity.
// If the same run identity is already active, it terminates the recorded
// process group from the state file and waits for the OS lock to be released.
func AcquireWatchRunLock(identity WatchRunIdentity, timeout time.Duration) (*WatchRunLock, bool, error) {
	baseDir, err := DefaultWatchRunBaseDir()
	if err != nil {
		return nil, false, err
	}
	return AcquireWatchRunLockInDir(baseDir, identity, timeout)
}

// AcquireWatchRunLockInDir is AcquireWatchRunLock with an explicit base
// directory so tests can exercise flock behavior without touching user state.
func AcquireWatchRunLockInDir(baseDir string, identity WatchRunIdentity, timeout time.Duration) (*WatchRunLock, bool, error) {
	identity = normalizeWatchRunIdentity(identity)
	if err := validateWatchRunIdentity(identity); err != nil {
		return nil, false, err
	}
	if baseDir == "" {
		return nil, false, fmt.Errorf("watch run base dir is empty")
	}

	paths := watchRunPaths(baseDir, identity)
	stoppedOtherOwners, err := stopOtherWatchRunOwners(baseDir, identity, paths.State, timeout)
	if err != nil {
		return nil, false, err
	}
	lock, err := tryAcquireWatchRunLock(paths.Lock, paths.State)
	if err == nil {
		if err := lock.writeState(identity); err != nil {
			lock.Close()
			return nil, false, err
		}
		return lock, stoppedOtherOwners, nil
	}
	if !errors.Is(err, errWatchRunLocked) {
		return nil, false, err
	}

	state, err := readWatchRunStateWithRetry(paths.State, 250*time.Millisecond)
	if err != nil {
		return nil, false, fmt.Errorf("dev watch lock is held but state is unreadable at %s: %w", paths.State, err)
	}
	if !sameWatchRunOwner(state.Identity, identity) {
		return nil, false, fmt.Errorf("dev watch lock state identity mismatch at %s", paths.State)
	}
	if state.PGID <= 0 {
		return nil, false, fmt.Errorf("dev watch lock state is missing a process group at %s", paths.State)
	}

	if err := signalProcessGroup(state.PGID, syscall.SIGTERM); err != nil {
		return nil, false, err
	}

	lock, err = waitForWatchRunLock(paths, identity, timeout)
	if err == nil {
		return lock, true, nil
	}
	if !errors.Is(err, errWatchRunLocked) {
		return nil, false, err
	}

	if err := signalProcessGroup(state.PGID, syscall.SIGKILL); err != nil {
		return nil, false, err
	}
	lock, err = waitForWatchRunLock(paths, identity, time.Second)
	if err != nil {
		if errors.Is(err, errWatchRunLocked) {
			return nil, false, fmt.Errorf("previous dev watch process group %d did not exit after SIGKILL; inspect %s before retrying", state.PGID, paths.Lock)
		}
		return nil, false, err
	}
	return lock, true, nil
}

// DefaultWatchRunBaseDir returns the shared location for dev watch lock files.
// Individual runs are separated by a hash of profile and mode.
func DefaultWatchRunBaseDir() (string, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return "", err
	}
	return filepath.Join(home, ".browseros-dev", "runs"), nil
}

// StopAllWatchProcesses terminates every recorded dev watch run.
func StopAllWatchProcesses(timeout time.Duration) (int, error) {
	baseDir, err := DefaultWatchRunBaseDir()
	if err != nil {
		return 0, err
	}
	return StopAllWatchProcessesInDir(baseDir, timeout)
}

// StopAllWatchProcessesInDir is StopAllWatchProcesses with an explicit state directory for tests.
func StopAllWatchProcessesInDir(baseDir string, timeout time.Duration) (int, error) {
	pgids, err := liveWatchRunPGIDs(baseDir)
	if err != nil {
		return 0, err
	}
	if len(pgids) == 0 {
		return 0, nil
	}

	for _, pgid := range pgids {
		if err := signalProcessGroup(pgid, syscall.SIGTERM); err != nil {
			return 0, err
		}
	}

	deadline := time.Now().Add(timeout)
	for {
		remaining := livePGIDs(pgids)
		if len(remaining) == 0 {
			return len(pgids), nil
		}
		if time.Now().After(deadline) {
			for _, pgid := range remaining {
				if err := signalProcessGroup(pgid, syscall.SIGKILL); err != nil {
					return 0, err
				}
			}
			return len(pgids), nil
		}
		time.Sleep(100 * time.Millisecond)
	}
}

// stopOtherWatchRunOwners tears down stale owners from older lock-key schemes.
func stopOtherWatchRunOwners(baseDir string, identity WatchRunIdentity, currentStatePath string, timeout time.Duration) (bool, error) {
	statePaths, err := filepath.Glob(filepath.Join(baseDir, "watch-*.json"))
	if err != nil {
		return false, err
	}
	currentStatePath = filepath.Clean(currentStatePath)
	seen := map[int]struct{}{}
	var pids []int
	for _, statePath := range statePaths {
		if filepath.Clean(statePath) == currentStatePath {
			continue
		}
		state, err := ReadWatchRunState(statePath)
		if err != nil || state.PID <= 0 || !sameWatchRunOwner(state.Identity, identity) || !processLive(state.PID) {
			continue
		}
		if _, ok := seen[state.PID]; ok {
			continue
		}
		seen[state.PID] = struct{}{}
		pids = append(pids, state.PID)
	}
	if len(pids) == 0 {
		return false, nil
	}
	sort.Ints(pids)
	for _, pid := range pids {
		if err := signalProcess(pid, syscall.SIGTERM); err != nil {
			return false, err
		}
	}

	deadline := time.Now().Add(timeout)
	for {
		remaining := livePIDs(pids)
		if len(remaining) == 0 {
			return true, nil
		}
		if time.Now().After(deadline) {
			for _, pid := range remaining {
				if err := signalProcess(pid, syscall.SIGKILL); err != nil {
					return false, err
				}
			}
			return true, nil
		}
		time.Sleep(100 * time.Millisecond)
	}
}

// KillBrowserProcessesForDevProfiles kills BrowserOS instances using temporary dev/test profiles.
func KillBrowserProcessesForDevProfiles(timeout time.Duration) (int, error) {
	return killBrowserProcesses([]string{"/tmp/browseros-dev"}, true, timeout)
}

// KillBrowserProcessesForUserDataDirs kills BrowserOS instances using the given user-data dirs.
func KillBrowserProcessesForUserDataDirs(userDataDirs []string, timeout time.Duration) (int, error) {
	return killBrowserProcesses(userDataDirs, false, timeout)
}

func killBrowserProcesses(userDataDirs []string, includeDevTempProfiles bool, timeout time.Duration) (int, error) {
	pids, err := currentBrowserProfilePIDs(userDataDirs, includeDevTempProfiles)
	if err != nil {
		return 0, err
	}
	if len(pids) == 0 {
		return 0, nil
	}
	for _, pid := range pids {
		if err := signalProcess(pid, syscall.SIGTERM); err != nil {
			return 0, err
		}
	}

	deadline := time.Now().Add(timeout)
	for {
		remaining, err := currentBrowserProfilePIDs(userDataDirs, includeDevTempProfiles)
		if err != nil {
			return 0, err
		}
		if len(remaining) == 0 {
			return len(pids), nil
		}
		if time.Now().After(deadline) {
			for _, pid := range remaining {
				if err := signalProcess(pid, syscall.SIGKILL); err != nil {
					return 0, err
				}
			}
			return len(pids), nil
		}
		time.Sleep(100 * time.Millisecond)
	}
}

func (l *WatchRunLock) Close() error {
	if l == nil || l.file == nil {
		return nil
	}

	// Keep the lock file path stable. Unlinking it during handoff can let
	// another opener lock a different inode while an owner still holds this one.
	removeErr := os.Remove(l.statePath)
	unlockErr := syscall.Flock(int(l.file.Fd()), syscall.LOCK_UN)
	closeErr := l.file.Close()
	l.file = nil
	if removeErr != nil && !os.IsNotExist(removeErr) {
		return removeErr
	}
	if unlockErr != nil {
		return unlockErr
	}
	return closeErr
}

// ReadWatchRunState reads the metadata used to terminate a previous owner.
// The state file is not the lock; it is only trusted after flock says a run is active.
func ReadWatchRunState(path string) (WatchRunState, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return WatchRunState{}, err
	}
	var state WatchRunState
	if err := json.Unmarshal(data, &state); err != nil {
		return WatchRunState{}, fmt.Errorf("parse watch run state: %w", err)
	}
	return state, nil
}

func readWatchRunStateWithRetry(path string, timeout time.Duration) (WatchRunState, error) {
	deadline := time.Now().Add(timeout)
	var lastErr error
	for {
		state, err := ReadWatchRunState(path)
		if err == nil {
			return state, nil
		}
		lastErr = err
		if time.Now().After(deadline) {
			return WatchRunState{}, lastErr
		}
		time.Sleep(50 * time.Millisecond)
	}
}

func liveWatchRunPGIDs(baseDir string) ([]int, error) {
	statePaths, err := filepath.Glob(filepath.Join(baseDir, "watch-*.json"))
	if err != nil {
		return nil, err
	}
	seen := map[int]struct{}{}
	for _, statePath := range statePaths {
		state, err := ReadWatchRunState(statePath)
		if err != nil || state.PGID <= 0 || !processGroupLive(state.PGID) {
			continue
		}
		seen[state.PGID] = struct{}{}
	}
	pgids := make([]int, 0, len(seen))
	for pgid := range seen {
		pgids = append(pgids, pgid)
	}
	sort.Ints(pgids)
	return pgids, nil
}

func livePGIDs(pgids []int) []int {
	remaining := make([]int, 0, len(pgids))
	for _, pgid := range pgids {
		if processGroupLive(pgid) {
			remaining = append(remaining, pgid)
		}
	}
	return remaining
}

func livePIDs(pids []int) []int {
	remaining := make([]int, 0, len(pids))
	for _, pid := range pids {
		if processLive(pid) {
			remaining = append(remaining, pid)
		}
	}
	return remaining
}

func processGroupLive(pgid int) bool {
	if pgid <= 0 {
		return false
	}
	err := syscall.Kill(-pgid, 0)
	return err == nil || err == syscall.EPERM
}

func processLive(pid int) bool {
	if pid <= 0 {
		return false
	}
	err := syscall.Kill(pid, 0)
	return err == nil || err == syscall.EPERM
}

func currentBrowserProfilePIDs(userDataDirs []string, includeDevTempProfiles bool) ([]int, error) {
	output, err := exec.Command("ps", "-axo", "pid=,pgid=,command=").Output()
	if err != nil {
		return nil, fmt.Errorf("listing processes: %w", err)
	}
	return browserProfilePIDsFromPSForUserDataDirs(string(output), userDataDirs, includeDevTempProfiles), nil
}

func browserProfilePIDsFromPS(output string) []int {
	return browserProfilePIDsFromPSForUserDataDirs(output, []string{"/tmp/browseros-dev"}, true)
}

func browserProfilePIDsFromPSForUserDataDirs(output string, userDataDirs []string, includeDevTempProfiles bool) []int {
	var pids []int
	for _, line := range strings.Split(output, "\n") {
		fields := strings.Fields(line)
		if len(fields) < 3 {
			continue
		}
		pid, err := strconv.Atoi(fields[0])
		if err != nil {
			continue
		}
		command := strings.Join(fields[2:], " ")
		if isBrowserProcessForUserDataDir(command, userDataDirs, includeDevTempProfiles) {
			pids = append(pids, pid)
		}
	}
	sort.Ints(pids)
	return pids
}

func isDevBrowserProcess(command string) bool {
	return isBrowserProcessForUserDataDir(command, []string{"/tmp/browseros-dev"}, true)
}

func isBrowserProcessForUserDataDir(command string, userDataDirs []string, includeDevTempProfiles bool) bool {
	if !strings.Contains(command, "BrowserOS.app/Contents/MacOS/BrowserOS") {
		return false
	}
	for _, dir := range userDataDirs {
		if dir == "" {
			continue
		}
		if strings.Contains(command, "--user-data-dir="+dir) {
			return true
		}
	}
	return includeDevTempProfiles &&
		(strings.Contains(command, "browseros-dev-") ||
			strings.Contains(command, "browseros-test-"))
}

func watchRunPaths(baseDir string, identity WatchRunIdentity) watchRunPathsResult {
	identity = normalizeWatchRunIdentity(identity)
	sum := sha256.Sum256([]byte(fmt.Sprintf("%s\x00%s",
		identity.Mode,
		identity.Profile,
	)))
	key := hex.EncodeToString(sum[:])
	return watchRunPathsResult{
		Lock:  filepath.Join(baseDir, "watch-"+key+".lock"),
		State: filepath.Join(baseDir, "watch-"+key+".json"),
	}
}

func normalizeWatchRunIdentity(identity WatchRunIdentity) WatchRunIdentity {
	identity.Profile = filepath.Clean(identity.Profile)
	return identity
}

func sameWatchRunOwner(a WatchRunIdentity, b WatchRunIdentity) bool {
	a = normalizeWatchRunIdentity(a)
	b = normalizeWatchRunIdentity(b)
	return a.Mode == b.Mode && a.Profile == b.Profile
}

func tryAcquireWatchRunLock(lockPath string, statePath string) (*WatchRunLock, error) {
	if err := os.MkdirAll(filepath.Dir(lockPath), 0o755); err != nil {
		return nil, err
	}
	file, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o644)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(file.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		file.Close()
		if errors.Is(err, syscall.EWOULDBLOCK) || errors.Is(err, syscall.EAGAIN) {
			return nil, errWatchRunLocked
		}
		return nil, err
	}
	return &WatchRunLock{file: file, statePath: statePath}, nil
}

func (l *WatchRunLock) writeState(identity WatchRunIdentity) error {
	pgid, err := syscall.Getpgid(0)
	if err != nil {
		return fmt.Errorf("reading current process group: %w", err)
	}
	state := WatchRunState{
		PID:       os.Getpid(),
		PGID:      pgid,
		StartedAt: time.Now(),
		Identity:  identity,
	}
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	tmp := l.statePath + ".tmp"
	if err := os.WriteFile(tmp, data, 0o644); err != nil {
		return err
	}
	return os.Rename(tmp, l.statePath)
}

func waitForWatchRunLock(paths watchRunPathsResult, identity WatchRunIdentity, timeout time.Duration) (*WatchRunLock, error) {
	deadline := time.Now().Add(timeout)
	for {
		lock, err := tryAcquireWatchRunLock(paths.Lock, paths.State)
		if err == nil {
			if err := lock.writeState(identity); err != nil {
				lock.Close()
				return nil, err
			}
			return lock, nil
		}
		if !errors.Is(err, errWatchRunLocked) {
			return nil, err
		}
		if time.Now().After(deadline) {
			return nil, errWatchRunLocked
		}
		time.Sleep(100 * time.Millisecond)
	}
}

func validateWatchRunIdentity(identity WatchRunIdentity) error {
	if identity.Mode == "" {
		return fmt.Errorf("watch run mode is empty")
	}
	if identity.Profile == "" {
		return fmt.Errorf("watch run profile is empty")
	}
	if !isValidTCPPort(identity.Ports.CDP) || !isValidTCPPort(identity.Ports.Server) || !isValidTCPPort(identity.Ports.Extension) {
		return fmt.Errorf("watch run ports are invalid: %+v", identity.Ports)
	}
	return nil
}

func isValidTCPPort(port int) bool {
	return port > 0 && port <= maxTCPPort
}

func signalProcessGroup(pgid int, signal syscall.Signal) error {
	if pgid <= 0 {
		return fmt.Errorf("invalid process group %d", pgid)
	}
	if err := syscall.Kill(-pgid, signal); err != nil && err != syscall.ESRCH {
		return fmt.Errorf("signaling process group %d: %w", pgid, err)
	}
	return nil
}

func signalProcess(pid int, signal syscall.Signal) error {
	if pid <= 0 {
		return fmt.Errorf("invalid process %d", pid)
	}
	if err := syscall.Kill(pid, signal); err != nil && err != syscall.ESRCH {
		return fmt.Errorf("signaling process %d: %w", pid, err)
	}
	return nil
}
