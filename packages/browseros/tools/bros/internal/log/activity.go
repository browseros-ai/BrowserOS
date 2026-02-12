package log

import (
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"bros/internal/patch"
)

type Logger struct {
	logFile string
}

func New(brosDir string) *Logger {
	return &Logger{
		logFile: filepath.Join(brosDir, "logs", "activity.log"),
	}
}

func (l *Logger) LogPush(base string, result *patch.PushResult) error {
	var b strings.Builder

	b.WriteString(divider('='))
	b.WriteString(fmt.Sprintf("PUSH  %s\n", time.Now().Format("2006-01-02 15:04:05")))
	b.WriteString(fmt.Sprintf("Base: %s\n", base))
	b.WriteString(divider('-'))

	for _, f := range result.Modified {
		b.WriteString(fmt.Sprintf("  M %s\n", f))
	}
	for _, f := range result.Added {
		b.WriteString(fmt.Sprintf("  A %s\n", f))
	}
	for _, f := range result.Deleted {
		b.WriteString(fmt.Sprintf("  D %s\n", f))
	}
	if len(result.Stale) > 0 {
		b.WriteString("Stale removed:\n")
		for _, f := range result.Stale {
			b.WriteString(fmt.Sprintf("  %s\n", f))
		}
	}

	b.WriteString(fmt.Sprintf("Summary: %d pushed (%d modified, %d added, %d deleted)",
		result.Total(), len(result.Modified), len(result.Added), len(result.Deleted)))
	if len(result.Stale) > 0 {
		b.WriteString(fmt.Sprintf(", %d stale removed", len(result.Stale)))
	}
	b.WriteString("\n\n")

	return l.append(b.String())
}

func (l *Logger) LogPull(base, repoRev string, result *patch.PullResult) error {
	var b strings.Builder

	b.WriteString(divider('='))
	b.WriteString(fmt.Sprintf("PULL  %s\n", time.Now().Format("2006-01-02 15:04:05")))
	b.WriteString(fmt.Sprintf("Base: %s\n", base))
	b.WriteString(fmt.Sprintf("Patches repo rev: %s\n", repoRev))
	b.WriteString(divider('-'))

	for _, f := range result.Applied {
		b.WriteString(fmt.Sprintf("  + %s\n", f))
	}
	for _, c := range result.Conflicts {
		b.WriteString(fmt.Sprintf("  x %s -> %s (hunk %d/%d failed)\n",
			c.File, c.RejectFile, c.HunksFailed, c.HunksTotal))
	}
	if len(result.Skipped) > 0 {
		b.WriteString(fmt.Sprintf("  ~ %d files skipped (already up to date)\n", len(result.Skipped)))
	}

	b.WriteString(fmt.Sprintf("Summary: %d applied, %d conflicts, %d skipped\n\n",
		len(result.Applied), len(result.Conflicts), len(result.Skipped)))

	return l.append(b.String())
}

func (l *Logger) LogClone(base string, result *patch.PullResult) error {
	var b strings.Builder

	b.WriteString(divider('='))
	b.WriteString(fmt.Sprintf("CLONE  %s\n", time.Now().Format("2006-01-02 15:04:05")))
	b.WriteString(fmt.Sprintf("Base: %s\n", base))
	b.WriteString(divider('-'))

	for _, f := range result.Applied {
		b.WriteString(fmt.Sprintf("  + %s\n", f))
	}
	for _, c := range result.Conflicts {
		b.WriteString(fmt.Sprintf("  x %s -> %s\n", c.File, c.RejectFile))
	}

	b.WriteString(fmt.Sprintf("Summary: %d applied, %d conflicts\n\n",
		len(result.Applied), len(result.Conflicts)))

	return l.append(b.String())
}

func (l *Logger) append(entry string) error {
	if err := os.MkdirAll(filepath.Dir(l.logFile), 0o755); err != nil {
		return err
	}

	f, err := os.OpenFile(l.logFile, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0o644)
	if err != nil {
		return err
	}
	defer f.Close()

	_, err = f.WriteString(entry)
	return err
}

func divider(ch byte) string {
	return strings.Repeat(string(ch), 50) + "\n"
}
