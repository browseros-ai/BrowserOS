package output

import (
	"bytes"
	"io"
	"os"
	"strings"
	"testing"

	"browseros-cli/mcp"
)

func TestActivePageFormatsPagesList(t *testing.T) {
	result := &mcp.ToolResult{
		StructuredContent: map[string]any{
			"pages": []any{
				map[string]any{"page": float64(1), "title": "First", "url": "https://first.example"},
				map[string]any{"page": float64(2), "title": "Second", "url": "https://second.example", "isActive": true},
			},
		},
	}

	out := captureStdout(t, func() {
		ActivePage(result)
	})

	if !strings.Contains(out, "Active page: 2") {
		t.Fatalf("ActivePage() output = %q, want active page 2", out)
	}
	if !strings.Contains(out, "Second") {
		t.Fatalf("ActivePage() output = %q, want active page title", out)
	}
}

func captureStdout(t *testing.T, run func()) string {
	t.Helper()

	original := os.Stdout
	reader, writer, err := os.Pipe()
	if err != nil {
		t.Fatalf("os.Pipe() error = %v", err)
	}
	os.Stdout = writer
	t.Cleanup(func() {
		os.Stdout = original
	})

	run()
	if err := writer.Close(); err != nil {
		t.Fatalf("writer.Close() error = %v", err)
	}

	var buf bytes.Buffer
	if _, err := io.Copy(&buf, reader); err != nil {
		t.Fatalf("io.Copy() error = %v", err)
	}
	return buf.String()
}
