package cmd

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"

	"browseros-cli/mcp"
)

var snapshotRefPattern = regexp.MustCompile(`\[ref=@?e([0-9]+)\]`)

// elementRef normalizes legacy numeric element IDs to compact snapshot refs.
func elementRef(raw string) (string, error) {
	ref := strings.TrimSpace(raw)
	if ref == "" {
		return "", fmt.Errorf("empty element ref")
	}
	ref = strings.TrimPrefix(ref, "@")
	if strings.HasPrefix(ref, "e") {
		if _, err := strconv.Atoi(strings.TrimPrefix(ref, "e")); err != nil {
			return "", fmt.Errorf("invalid element ref: %s", raw)
		}
		return ref, nil
	}
	if _, err := strconv.Atoi(ref); err != nil {
		return "", fmt.Errorf("invalid element ref: %s", raw)
	}
	return "e" + ref, nil
}

func displayElementRefs(text string) string {
	return snapshotRefPattern.ReplaceAllString(text, `[ref=@e$1]`)
}

// browserRunValue runs compact-tool server JavaScript and returns its structured value.
func browserRunValue(c *mcp.Client, code string) (*mcp.ToolResult, any, error) {
	result, err := c.CallTool("run", map[string]any{"code": code})
	if err != nil {
		return result, nil, err
	}
	value, ok := result.StructuredContent["value"]
	if !ok {
		return result, nil, fmt.Errorf("run did not return a structured value")
	}
	return result, value, nil
}

func textResult(text string, structured map[string]any) *mcp.ToolResult {
	return &mcp.ToolResult{
		Content:           []mcp.ContentItem{{Type: "text", Text: text}},
		StructuredContent: structured,
	}
}

func jsLiteral(v any) string {
	data, err := json.Marshal(v)
	if err != nil {
		return "null"
	}
	return string(data)
}

func valueMap(v any) (map[string]any, bool) {
	m, ok := v.(map[string]any)
	return m, ok
}

func resultData(value any) map[string]any {
	if data, ok := valueMap(value); ok {
		return data
	}
	return map[string]any{"value": value}
}

func valueSlice(v any) []any {
	items, ok := v.([]any)
	if !ok {
		return nil
	}
	return items
}

func stringValue(v any) string {
	if s, ok := v.(string); ok {
		return s
	}
	return ""
}

func numberValue(v any) int {
	switch n := v.(type) {
	case int:
		return n
	case int32:
		return int(n)
	case int64:
		return int(n)
	case float64:
		return int(n)
	case json.Number:
		i, _ := n.Int64()
		return int(i)
	default:
		return 0
	}
}

func copyLocalFile(src, dst string) error {
	in, err := os.Open(src)
	if err != nil {
		return err
	}
	defer in.Close()

	if err := os.MkdirAll(filepath.Dir(dst), 0755); err != nil {
		return err
	}

	out, err := os.Create(dst)
	if err != nil {
		return err
	}

	if _, err := io.Copy(out, in); err != nil {
		_ = out.Close()
		return err
	}
	return out.Close()
}

// sanitizeDownloadFilename keeps browser-provided names inside the requested directory.
func sanitizeDownloadFilename(filename string) (string, error) {
	name := strings.TrimSpace(filename)
	if name == "" || name == "." || name == ".." {
		return "", fmt.Errorf("unsafe download filename: %q", filename)
	}
	if filepath.IsAbs(name) || filepath.VolumeName(name) != "" || filepath.Base(name) != name || strings.ContainsAny(name, `/\`) {
		return "", fmt.Errorf("unsafe download filename: %q", filename)
	}
	return name, nil
}

func copyDownloadFile(src, dir, filename string) (string, error) {
	safeName, err := sanitizeDownloadFilename(filename)
	if err != nil {
		return "", err
	}
	if err := os.MkdirAll(dir, 0755); err != nil {
		return "", err
	}

	in, err := os.Open(src)
	if err != nil {
		return "", err
	}
	defer in.Close()

	ext := filepath.Ext(safeName)
	base := strings.TrimSuffix(safeName, ext)
	if base == "" {
		base = safeName
		ext = ""
	}
	for i := 0; i < 1000; i++ {
		name := safeName
		if i > 0 {
			name = fmt.Sprintf("%s-%d%s", base, i, ext)
		}
		dst := filepath.Join(dir, name)
		out, err := os.OpenFile(dst, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0644)
		if os.IsExist(err) {
			continue
		}
		if err != nil {
			return "", err
		}
		if _, err := io.Copy(out, in); err != nil {
			_ = out.Close()
			_ = os.Remove(dst)
			return "", err
		}
		if err := out.Close(); err != nil {
			_ = os.Remove(dst)
			return "", err
		}
		return dst, nil
	}
	return "", fmt.Errorf("no available destination for %q", safeName)
}
