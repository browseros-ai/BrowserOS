package proc

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"
	"sync"
	"time"

	"github.com/fatih/color"
)

const LogMaxAge = 24 * time.Hour

type Tag struct {
	Name  string
	Color *color.Color
}

type LogFile struct {
	Name    string
	Path    string
	Size    int64
	ModTime time.Time
}

type LineHandler func(tag Tag, stream string, line string)

var (
	TagBuild   = Tag{"build", color.New(color.FgYellow)}
	TagAgent   = Tag{"agent", color.New(color.FgMagenta)}
	TagServer  = Tag{"server", color.New(color.FgCyan)}
	TagBrowser = Tag{"browser", color.New(color.FgBlue)}
	TagInfo    = Tag{"info", color.New(color.FgGreen)}
	TagTest    = Tag{"test", color.New(color.FgWhite)}

	ErrorColor = color.New(color.FgRed)
	WarnColor  = color.New(color.FgYellow)
	BoldColor  = color.New(color.Bold)
	DimColor   = color.New(color.Faint)

	ansiPattern = regexp.MustCompile(`\x1b\[[0-9;]*m`)
)

func LogMsg(t Tag, msg string) {
	logMsg(t, msg, os.Stdout, nil, nil)
}

func LogMsgf(t Tag, format string, args ...any) {
	LogMsg(t, fmt.Sprintf(format, args...))
}

func LogMsgTee(t Tag, msg string, file io.Writer, fileMu *sync.Mutex) {
	logMsg(t, msg, os.Stdout, file, fileMu)
}

func StreamLines(r io.Reader, t Tag) {
	StreamLinesWithHandler(r, t, "", nil)
}

func StreamLinesWithHandler(r io.Reader, t Tag, stream string, handler LineHandler) {
	streamLinesWithHandler(r, t, stream, os.Stdout, nil, nil, handler)
}

func OpenLogFile(logDir string, name string, now time.Time) (*os.File, string, error) {
	if err := os.MkdirAll(logDir, 0755); err != nil {
		return nil, "", err
	}
	path := filepath.Join(logDir, name)
	if err := rotateLogIfNeeded(path, now); err != nil {
		return nil, "", err
	}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_APPEND|os.O_WRONLY, 0644)
	if err != nil {
		return nil, "", err
	}
	return file, path, nil
}

func ListLogFiles(logDir string) ([]LogFile, error) {
	entries, err := os.ReadDir(logDir)
	if os.IsNotExist(err) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}

	files := []LogFile{}
	for _, entry := range entries {
		if !strings.HasSuffix(entry.Name(), ".log") && !strings.HasSuffix(entry.Name(), ".log.old") {
			continue
		}
		info, err := entry.Info()
		if err != nil {
			return nil, err
		}
		if !info.Mode().IsRegular() {
			continue
		}
		files = append(files, LogFile{
			Name:    entry.Name(),
			Path:    filepath.Join(logDir, entry.Name()),
			Size:    info.Size(),
			ModTime: info.ModTime(),
		})
	}
	sort.Slice(files, func(i, j int) bool {
		return files[i].Name < files[j].Name
	})
	return files, nil
}

func rotateLogIfNeeded(logPath string, now time.Time) error {
	info, err := os.Stat(logPath)
	if os.IsNotExist(err) {
		return nil
	}
	if err != nil {
		return err
	}
	if now.Sub(info.ModTime()) <= LogMaxAge {
		return nil
	}
	backupPath := logPath + ".old"
	if err := os.Remove(backupPath); err != nil && !os.IsNotExist(err) {
		return err
	}
	return os.Rename(logPath, backupPath)
}

func streamLines(r io.Reader, t Tag, terminal io.Writer, file io.Writer, fileMu *sync.Mutex) {
	streamLinesWithHandler(r, t, "", terminal, file, fileMu, nil)
}

func streamLinesWithHandler(r io.Reader, t Tag, stream string, terminal io.Writer, file io.Writer, fileMu *sync.Mutex, handler LineHandler) {
	scanner := bufio.NewScanner(r)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for scanner.Scan() {
		line := scanner.Text()
		if line != "" {
			logMsg(t, line, terminal, file, fileMu)
			if handler != nil {
				handler(t, stream, line)
			}
		}
	}
	if err := scanner.Err(); err != nil {
		logMsg(t, fmt.Sprintf("log stream error: %v", err), terminal, file, fileMu)
	}
}

func logMsg(t Tag, msg string, terminal io.Writer, file io.Writer, fileMu *sync.Mutex) {
	if fileMu != nil {
		fileMu.Lock()
		defer fileMu.Unlock()
	}
	fmt.Fprintf(terminal, "%s %s\n", t.Color.Sprintf("[%s]", t.Name), msg)
	if file == nil {
		return
	}
	fmt.Fprintf(file, "[%s] %s\n", t.Name, ansiPattern.ReplaceAllString(msg, ""))
}
