package cmd

import (
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"testing"
)

func TestCLIDoesNotCallLegacyBrowserTools(t *testing.T) {
	legacyOnlyToolNames := map[string]bool{
		"get_bookmarks":         true,
		"get_dom":               true,
		"search_history":        true,
		"click":                 true,
		"list_pages":            true,
		"save_pdf":              true,
		"take_snapshot":         true,
		"group_tabs":            true,
		"list_windows":          true,
		"create_window":         true,
		"create_hidden_window":  true,
		"close_window":          true,
		"activate_window":       true,
		"set_window_visibility": true,
	}
	callToolLiteral := regexp.MustCompile(`CallTool\("([^"]+)"`)

	for _, dir := range []string{".", "../mcp"} {
		entries, err := os.ReadDir(dir)
		if err != nil {
			t.Fatalf("os.ReadDir(%q) error = %v", dir, err)
		}
		for _, entry := range entries {
			if entry.IsDir() || filepath.Ext(entry.Name()) != ".go" || strings.HasSuffix(entry.Name(), "_test.go") {
				continue
			}
			path := filepath.Join(dir, entry.Name())
			data, err := os.ReadFile(path)
			if err != nil {
				t.Fatalf("os.ReadFile(%q) error = %v", path, err)
			}
			for _, match := range callToolLiteral.FindAllSubmatch(data, -1) {
				name := string(match[1])
				if legacyOnlyToolNames[name] {
					t.Fatalf("%s calls legacy browser tool %q", path, name)
				}
			}
		}
	}
}
