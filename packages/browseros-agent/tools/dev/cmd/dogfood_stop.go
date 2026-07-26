package cmd

import (
	"bufio"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"syscall"
	"time"
)

const dogfoodStopTimeout = 10 * time.Second

type dogfoodRunState struct {
	PID        int    `json:"pid"`
	Mode       string `json:"mode"`
	SocketPath string `json:"socket_path"`
	LogPath    string `json:"log_path"`
}

type dogfoodIPCRequest struct {
	Command string `json:"command"`
}

type dogfoodIPCResponse struct {
	OK    bool   `json:"ok"`
	Error string `json:"error,omitempty"`
}

func ensureTargetStopped(out io.Writer, target resetTarget) error {
	if target.Dogfood == nil {
		return nil
	}
	return stopDogfoodRun(out, *target.Dogfood, dogfoodStopTimeout)
}

func stopDogfoodRun(out io.Writer, target dogfoodRuntimeTarget, timeout time.Duration) error {
	active, err := dogfoodRunActive(target.LockPath)
	if err != nil {
		return err
	}
	if !active {
		cleanupDogfoodRunFilesWithWarning(out, target)
		return nil
	}

	fmt.Fprintln(out, labelStyle.Sprint("Stopping dogfood run first."))
	if err := stopDogfoodDaemon(target); err == nil {
		if stopped, err := waitForDogfoodStopped(out, target, timeout); err != nil {
			return err
		} else if stopped {
			fmt.Fprintln(out, successStyle.Sprint("Dogfood stopped."))
			return nil
		}
	}

	state, err := readDogfoodRunState(target.StatePath)
	if err != nil {
		return fmt.Errorf("dogfood is running but state is unreadable at %s: %w", target.StatePath, err)
	}
	if state.PID <= 0 {
		return fmt.Errorf("dogfood is running but state has no pid at %s", target.StatePath)
	}
	if err := signalDogfoodPID(state.PID, syscall.SIGTERM); err != nil {
		return err
	}
	if stopped, err := waitForDogfoodStopped(out, target, timeout); err != nil {
		return err
	} else if stopped {
		fmt.Fprintln(out, successStyle.Sprint("Dogfood stopped."))
		return nil
	}
	if err := signalDogfoodPID(state.PID, syscall.SIGKILL); err != nil {
		return err
	}
	if stopped, err := waitForDogfoodStopped(out, target, time.Second); err != nil {
		return err
	} else if stopped {
		fmt.Fprintln(out, successStyle.Sprint("Dogfood force-stopped."))
		return nil
	}
	return fmt.Errorf("dogfood is still running; stop it manually before cleanup/reset")
}

func stopDogfoodDaemon(target dogfoodRuntimeTarget) error {
	socketPath := target.SocketPath
	if state, err := readDogfoodRunState(target.StatePath); err == nil && state.SocketPath != "" {
		socketPath = state.SocketPath
	}
	conn, err := net.DialTimeout("unix", socketPath, 700*time.Millisecond)
	if err != nil {
		return err
	}
	defer conn.Close()

	data, err := json.Marshal(dogfoodIPCRequest{Command: "stop"})
	if err != nil {
		return err
	}
	data = append(data, '\n')
	if _, err := conn.Write(data); err != nil {
		return err
	}
	_ = conn.SetReadDeadline(time.Now().Add(2 * time.Second))
	scanner := bufio.NewScanner(conn)
	if !scanner.Scan() {
		if err := scanner.Err(); err != nil {
			return err
		}
		return errors.New("dogfood daemon closed connection without response")
	}
	var response dogfoodIPCResponse
	if err := json.Unmarshal(scanner.Bytes(), &response); err != nil {
		return err
	}
	if response.Error != "" {
		return errors.New(response.Error)
	}
	if !response.OK {
		return errors.New("dogfood daemon did not accept stop request")
	}
	return nil
}

func waitForDogfoodStopped(out io.Writer, target dogfoodRuntimeTarget, timeout time.Duration) (bool, error) {
	deadline := time.Now().Add(timeout)
	for {
		active, err := dogfoodRunActive(target.LockPath)
		if err != nil {
			return false, err
		}
		if !active {
			cleanupDogfoodRunFilesWithWarning(out, target)
			return true, nil
		}
		if time.Now().After(deadline) {
			return false, nil
		}
		time.Sleep(100 * time.Millisecond)
	}
}

func dogfoodRunActive(lockPath string) (bool, error) {
	file, err := os.OpenFile(lockPath, os.O_CREATE|os.O_RDWR, 0o644)
	if err != nil {
		return false, err
	}
	defer file.Close()
	if err := syscall.Flock(int(file.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		if errors.Is(err, syscall.EWOULDBLOCK) || errors.Is(err, syscall.EAGAIN) {
			return true, nil
		}
		return false, err
	}
	return false, syscall.Flock(int(file.Fd()), syscall.LOCK_UN)
}

func readDogfoodRunState(path string) (dogfoodRunState, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return dogfoodRunState{}, err
	}
	var state dogfoodRunState
	if err := json.Unmarshal(data, &state); err != nil {
		return dogfoodRunState{}, err
	}
	return state, nil
}

func signalDogfoodPID(pid int, sig syscall.Signal) error {
	if pid <= 0 {
		return fmt.Errorf("invalid dogfood pid %d", pid)
	}
	if err := syscall.Kill(pid, sig); err != nil && err != syscall.ESRCH {
		return err
	}
	return nil
}

func cleanupDogfoodRunFilesWithWarning(out io.Writer, target dogfoodRuntimeTarget) {
	if err := cleanupDogfoodRunFiles(target); err != nil {
		fmt.Fprintf(out, "%s could not remove dogfood run files: %v\n", warnStyle.Sprint("Warning:"), err)
	}
}

func cleanupDogfoodRunFiles(target dogfoodRuntimeTarget) error {
	if err := os.Remove(target.SocketPath); err != nil && !os.IsNotExist(err) {
		return err
	}
	if err := os.Remove(target.StatePath); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}
