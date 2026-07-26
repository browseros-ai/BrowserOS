package runlog

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"time"
)

type Entry struct {
	Time   time.Time `json:"time"`
	Tag    string    `json:"tag"`
	Stream string    `json:"stream"`
	Line   string    `json:"line"`
}

type Writer struct {
	mu   sync.Mutex
	file *os.File
	enc  *json.Encoder
}

func NewWriter(path string) (*Writer, error) {
	if err := os.MkdirAll(filepath.Dir(path), 0755); err != nil {
		return nil, err
	}
	file, err := os.OpenFile(path, os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		return nil, err
	}
	return &Writer{file: file, enc: json.NewEncoder(file)}, nil
}

func (w *Writer) Append(tag string, stream string, line string) error {
	if w == nil {
		return nil
	}
	w.mu.Lock()
	defer w.mu.Unlock()
	return w.enc.Encode(Entry{
		Time:   time.Now(),
		Tag:    tag,
		Stream: stream,
		Line:   line,
	})
}

func (w *Writer) Close() error {
	if w == nil || w.file == nil {
		return nil
	}
	return w.file.Close()
}

func ReadLast(path string, maxLines int, filter string) ([]Entry, error) {
	normalized, err := NormalizeFilter(filter)
	if err != nil {
		return nil, err
	}
	file, err := os.Open(path)
	if err != nil {
		return nil, err
	}
	defer file.Close()

	var entries []Entry
	scanner := bufio.NewScanner(file)
	scanner.Buffer(make([]byte, 0, 64*1024), 1024*1024)
	for scanner.Scan() {
		entry, ok := parseLine(scanner.Text(), normalized)
		if !ok {
			continue
		}
		entries = append(entries, entry)
		if maxLines > 0 && len(entries) > maxLines {
			entries = entries[len(entries)-maxLines:]
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	return entries, nil
}

func Follow(path string, filter string, onEntry func(Entry)) error {
	return FollowWithContext(context.Background(), path, filter, onEntry)
}

func FollowWithContext(ctx context.Context, path string, filter string, onEntry func(Entry)) error {
	return followWithContext(ctx, path, filter, true, onEntry)
}

func FollowFromStartWithContext(ctx context.Context, path string, filter string, onEntry func(Entry)) error {
	return followWithContext(ctx, path, filter, false, onEntry)
}

func followWithContext(ctx context.Context, path string, filter string, seekEnd bool, onEntry func(Entry)) error {
	normalized, err := NormalizeFilter(filter)
	if err != nil {
		return err
	}
	file, err := os.Open(path)
	if err != nil {
		return err
	}
	defer file.Close()

	if seekEnd {
		if _, err := file.Seek(0, io.SeekEnd); err != nil {
			return err
		}
	}
	reader := bufio.NewReader(file)
	for {
		if ctx.Err() != nil {
			return nil
		}
		line, err := reader.ReadString('\n')
		if err != nil {
			if err == io.EOF {
				nextFile, replaced, reopenErr := reopenIfReplaced(path, file)
				if reopenErr != nil {
					return reopenErr
				}
				if replaced {
					file.Close()
					file = nextFile
					reader = bufio.NewReader(file)
					continue
				}
				select {
				case <-ctx.Done():
					return nil
				case <-time.After(200 * time.Millisecond):
				}
				continue
			}
			return err
		}
		entry, ok := parseLine(strings.TrimRight(line, "\r\n"), normalized)
		if ok {
			onEntry(entry)
		}
	}
}

func reopenIfReplaced(path string, current *os.File) (*os.File, bool, error) {
	currentInfo, currentErr := current.Stat()
	pathInfo, pathErr := os.Stat(path)
	if os.IsNotExist(pathErr) {
		return nil, false, nil
	}
	if pathErr != nil {
		return nil, false, pathErr
	}
	if currentErr == nil && os.SameFile(currentInfo, pathInfo) {
		return nil, false, nil
	}
	next, err := os.Open(path)
	if err != nil {
		return nil, false, err
	}
	return next, true, nil
}

func Format(entry Entry) string {
	return fmt.Sprintf("%s [%s] %s", entry.Time.Format("15:04:05"), displayTag(entry.Tag), entry.Line)
}

func NormalizeFilter(filter string) (string, error) {
	switch strings.ToLower(strings.TrimSpace(filter)) {
	case "", "all":
		return "", nil
	case "browser", "chromium":
		return "browser", nil
	case "server":
		return "server", nil
	case "daemon":
		return "daemon", nil
	default:
		return "", fmt.Errorf("unknown log filter %q; use daemon, chromium, or server", filter)
	}
}

func parseLine(line string, filter string) (Entry, bool) {
	var entry Entry
	if strings.TrimSpace(line) == "" {
		return Entry{}, false
	}
	if err := json.Unmarshal([]byte(line), &entry); err != nil {
		return Entry{}, false
	}
	if filter != "" && entry.Tag != filter {
		return Entry{}, false
	}
	return entry, true
}

func displayTag(tag string) string {
	if tag == "browser" {
		return "chromium"
	}
	return tag
}
