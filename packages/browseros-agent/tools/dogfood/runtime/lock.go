package runtime

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"syscall"
	"time"
)

var ErrAlreadyRunning = errors.New("browseros-dogfood is already running")

type Lock struct {
	file *os.File
	path string
}

type RunState struct {
	PID        int       `json:"pid"`
	Mode       string    `json:"mode"`
	StartedAt  time.Time `json:"started_at"`
	SocketPath string    `json:"socket_path"`
	LogPath    string    `json:"log_path"`
}

func AcquireLock(path string) (*Lock, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return nil, err
	}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_RDWR, 0644)
	if err != nil {
		return nil, err
	}
	if err := syscall.Flock(int(file.Fd()), syscall.LOCK_EX|syscall.LOCK_NB); err != nil {
		file.Close()
		if errors.Is(err, syscall.EWOULDBLOCK) || errors.Is(err, syscall.EAGAIN) {
			return nil, ErrAlreadyRunning
		}
		return nil, err
	}
	return &Lock{file: file, path: path}, nil
}

func (l *Lock) Close() error {
	if l == nil || l.file == nil {
		return nil
	}
	unlockErr := syscall.Flock(int(l.file.Fd()), syscall.LOCK_UN)
	closeErr := l.file.Close()
	l.file = nil
	if unlockErr != nil {
		return unlockErr
	}
	return closeErr
}

func WriteRunState(path string, state RunState) error {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return err
	}
	data, err := json.MarshalIndent(state, "", "  ")
	if err != nil {
		return err
	}
	data = append(data, '\n')
	tmp := path + ".tmp"
	if err := os.WriteFile(tmp, data, 0644); err != nil {
		return err
	}
	return os.Rename(tmp, path)
}

func ReadRunState(path string) (RunState, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return RunState{}, err
	}
	var state RunState
	if err := json.Unmarshal(data, &state); err != nil {
		return RunState{}, fmt.Errorf("parse run state: %w", err)
	}
	return state, nil
}

func CleanupStaleRunFiles(statePath string) error {
	state, err := ReadRunState(statePath)
	if err != nil && !os.IsNotExist(err) {
		return err
	}
	if err == nil && state.SocketPath != "" {
		if removeErr := os.Remove(state.SocketPath); removeErr != nil && !os.IsNotExist(removeErr) {
			return removeErr
		}
	}
	if err := os.Remove(statePath); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}
