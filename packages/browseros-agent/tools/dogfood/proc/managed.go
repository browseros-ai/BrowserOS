package proc

import (
	"context"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync"
	"syscall"
	"time"
)

type ProcConfig struct {
	Tag         Tag
	Dir         string
	Env         []string
	Restart     bool
	Cmd         []string
	LogPath     string
	LineHandler LineHandler
}

type ManagedProc struct {
	Cfg    ProcConfig
	cancel context.CancelFunc
	mu     sync.Mutex
	proc   *os.Process
	exited chan struct{}
}

func StartManaged(ctx context.Context, wg *sync.WaitGroup, cfg ProcConfig) *ManagedProc {
	procCtx, procCancel := context.WithCancel(ctx)
	mp := &ManagedProc{
		Cfg:    cfg,
		cancel: procCancel,
		exited: make(chan struct{}),
	}

	wg.Add(1)
	go func() {
		defer wg.Done()
		mp.run(procCtx)
	}()

	return mp
}

func (mp *ManagedProc) run(ctx context.Context) {
	var logFile *os.File
	var logMu sync.Mutex
	if mp.Cfg.LogPath != "" {
		file, path, err := OpenLogFile(filepath.Dir(mp.Cfg.LogPath), filepath.Base(mp.Cfg.LogPath), time.Now())
		if err != nil {
			LogMsg(mp.Cfg.Tag, WarnColor.Sprintf("File logging disabled: %v", err))
		} else {
			logFile = file
			defer logFile.Close()
			LogMsgTee(mp.Cfg.Tag, "Writing log file: "+path, logFile, &logMu)
		}
	}
	log := func(msg string) {
		LogMsgTee(mp.Cfg.Tag, msg, logFile, &logMu)
	}

	for {
		if ctx.Err() != nil {
			return
		}

		log(fmt.Sprintf("Starting: %s", DimColor.Sprint(strings.Join(mp.Cfg.Cmd, " "))))

		cmd := exec.Command(mp.Cfg.Cmd[0], mp.Cfg.Cmd[1:]...)
		cmd.Dir = mp.Cfg.Dir
		if mp.Cfg.Env != nil {
			cmd.Env = mp.Cfg.Env
		}
		cmd.SysProcAttr = &syscall.SysProcAttr{Setpgid: true}

		stdout, _ := cmd.StdoutPipe()
		stderr, _ := cmd.StderrPipe()

		if err := cmd.Start(); err != nil {
			log(ErrorColor.Sprintf("Error starting: %v", err))
			if !mp.Cfg.Restart || ctx.Err() != nil {
				return
			}
			time.Sleep(time.Second)
			continue
		}

		exited := make(chan struct{})
		mp.mu.Lock()
		mp.proc = cmd.Process
		mp.exited = exited
		cancelled := ctx.Err() != nil
		mp.mu.Unlock()
		if cancelled {
			_ = syscall.Kill(-cmd.Process.Pid, syscall.SIGTERM)
		}

		var streamWg sync.WaitGroup
		streamWg.Add(2)
		go func() {
			defer streamWg.Done()
			streamLinesWithHandler(stdout, mp.Cfg.Tag, "stdout", os.Stdout, logFile, &logMu, mp.Cfg.LineHandler)
		}()
		go func() {
			defer streamWg.Done()
			streamLinesWithHandler(stderr, mp.Cfg.Tag, "stderr", os.Stdout, logFile, &logMu, mp.Cfg.LineHandler)
		}()

		streamWg.Wait()
		_ = cmd.Wait()

		mp.mu.Lock()
		mp.proc = nil
		close(mp.exited)
		mp.mu.Unlock()

		if ctx.Err() != nil {
			return
		}

		exitCode := cmd.ProcessState.ExitCode()
		if exitCode != 0 {
			log(ErrorColor.Sprintf("Process exited with code %d", exitCode))
		} else {
			log("Process exited cleanly")
		}

		if !mp.Cfg.Restart {
			return
		}

		log(WarnColor.Sprint("Restarting in 1s..."))
		select {
		case <-ctx.Done():
			return
		case <-time.After(time.Second):
		}
	}
}

func (mp *ManagedProc) Stop() {
	mp.cancel()
	mp.mu.Lock()
	proc := mp.proc
	exited := mp.exited
	mp.mu.Unlock()

	if proc != nil {
		_ = syscall.Kill(-proc.Pid, syscall.SIGTERM)
		select {
		case <-exited:
		case <-time.After(5 * time.Second):
			_ = syscall.Kill(-proc.Pid, syscall.SIGKILL)
			select {
			case <-exited:
			case <-time.After(3 * time.Second):
				LogMsg(mp.Cfg.Tag, WarnColor.Sprint("Process did not exit after SIGKILL, giving up"))
			}
		}
	}
}

func (mp *ManagedProc) ForceKill() {
	mp.mu.Lock()
	proc := mp.proc
	mp.mu.Unlock()

	if proc != nil {
		_ = syscall.Kill(-proc.Pid, syscall.SIGKILL)
	}
}
