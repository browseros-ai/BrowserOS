package cmd

import (
	"encoding/base64"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"browseros-cli/mcp"

	"github.com/spf13/cobra"
)

func TestCompactToolMappings(t *testing.T) {
	tests := []struct {
		name string
		got  map[string]any
		want map[string]any
	}{
		{
			name: "click",
			got:  clickToolArgs(7, "e12", "right", 2),
			want: map[string]any{
				"page":       7,
				"kind":       "click",
				"ref":        "e12",
				"button":     "right",
				"clickCount": 2,
			},
		},
		{
			name: "click at",
			got:  clickAtToolArgs(7, 10, 20),
			want: map[string]any{
				"page": 7,
				"kind": "click_at",
				"x":    10,
				"y":    20,
			},
		},
		{
			name: "list tabs",
			got:  tabsListToolArgs(),
			want: map[string]any{"action": "list"},
		},
		{
			name: "active tab",
			got:  tabsActiveToolArgs(),
			want: map[string]any{"action": "active"},
		},
		{
			name: "open tab",
			got:  openTabsToolArgs("https://example.com", true, false),
			want: map[string]any{
				"action":     "new",
				"url":        "https://example.com",
				"hidden":     true,
				"background": false,
			},
		},
		{
			name: "pdf",
			got:  pdfToolArgs(7),
			want: map[string]any{"page": 7},
		},
		{
			name: "diff",
			got:  diffToolArgs(7),
			want: map[string]any{"page": 7},
		},
		{
			name: "download",
			got:  downloadToolArgs(7, "e12"),
			want: map[string]any{
				"page": 7,
				"ref":  "e12",
			},
		},
		{
			name: "fill",
			got:  fillToolArgs(7, "e12", "hello", true),
			want: map[string]any{
				"page":  7,
				"kind":  "fill",
				"ref":   "e12",
				"value": "hello",
				"clear": true,
			},
		},
		{
			name: "fill without clear",
			got:  fillToolArgs(7, "e12", "hello", false),
			want: map[string]any{
				"page":  7,
				"kind":  "fill",
				"ref":   "e12",
				"value": "hello",
				"clear": false,
			},
		},
		{
			name: "press",
			got:  pressToolArgs(7, "Enter"),
			want: map[string]any{
				"page": 7,
				"kind": "press",
				"key":  "Enter",
			},
		},
		{
			name: "type",
			got:  typeToolArgs(7, "hello"),
			want: map[string]any{
				"page": 7,
				"kind": "type",
				"text": "hello",
			},
		},
		{
			name: "read markdown",
			got:  readToolArgs(7, readOptions{format: "markdown", includeLinks: true}),
			want: map[string]any{
				"page":         7,
				"format":       "markdown",
				"includeLinks": true,
			},
		},
		{
			name: "grep content",
			got:  grepToolArgs(7, "Example", "content", 5),
			want: map[string]any{
				"page":    7,
				"pattern": "Example",
				"over":    "content",
				"limit":   5,
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if !reflect.DeepEqual(tt.got, tt.want) {
				t.Fatalf("mapping = %#v, want %#v", tt.got, tt.want)
			}
		})
	}
}

func TestElementRefAcceptsCopyPasteForms(t *testing.T) {
	for _, raw := range []string{"@e12", "e12", "12"} {
		t.Run(raw, func(t *testing.T) {
			got, err := elementRef(raw)
			if err != nil {
				t.Fatalf("elementRef(%q) error = %v", raw, err)
			}
			if got != "e12" {
				t.Fatalf("elementRef(%q) = %q, want e12", raw, got)
			}
		})
	}
}

func TestDisplayElementRefsPrefersAtRefs(t *testing.T) {
	got := displayElementRefs("- button \"Buy\" [ref=e12]\n- input [ref=e3]")
	want := "- button \"Buy\" [ref=@e12]\n- input [ref=@e3]"
	if got != want {
		t.Fatalf("displayElementRefs() = %q, want %q", got, want)
	}
}

func TestFindMatchesTextAndBuildsClick(t *testing.T) {
	lines := []string{
		`- button "Add to Cart" [ref=e12]`,
		`- button "Add to Cart" [ref=e13]`,
	}
	query := findQuery{mode: "text", text: "add to cart", nth: 1}

	matches := findMatches(lines, query)
	if len(matches) != 2 {
		t.Fatalf("matches = %d, want 2", len(matches))
	}
	selected, err := selectFindMatch(matches, query)
	if err != nil {
		t.Fatalf("selectFindMatch() error = %v", err)
	}
	if selected.ref != "e12" {
		t.Fatalf("selected ref = %q, want e12", selected.ref)
	}

	calls, err := findActionCalls(7, selected, findAction{kind: "click"})
	if err != nil {
		t.Fatalf("findActionCalls() error = %v", err)
	}
	want := []toolCall{{name: "act", args: map[string]any{"page": 7, "kind": "click", "ref": "e12"}}}
	if !reflect.DeepEqual(calls, want) {
		t.Fatalf("calls = %#v, want %#v", calls, want)
	}
}

func TestFindMatchesRoleNameNth(t *testing.T) {
	lines := []string{
		`- link "Add to Cart details" [ref=e4]`,
		`- button "Add to Cart" [ref=e12]`,
		`- button "Add to Cart" [ref=e13]`,
	}
	query := findQuery{mode: "role", role: "button", name: "Add to Cart", nth: 2}

	selected, err := selectFindMatch(findMatches(lines, query), query)
	if err != nil {
		t.Fatalf("selectFindMatch() error = %v", err)
	}
	if selected.ref != "e13" {
		t.Fatalf("selected ref = %q, want e13", selected.ref)
	}
}

func TestGrepMatchLinesUsesStructuredMatches(t *testing.T) {
	result := &mcp.ToolResult{
		Content: []mcp.ContentItem{{Type: "text", Text: "[UNTRUSTED_PAGE_CONTENT]\nignored\n[END_UNTRUSTED_PAGE_CONTENT]"}},
		StructuredContent: map[string]any{
			"matches": []any{
				`- button "Add to Cart" [ref=e12]`,
				`- button "Add to Cart" [ref=e13]`,
			},
		},
	}

	lines, err := grepMatchLines(result)
	if err != nil {
		t.Fatalf("grepMatchLines() error = %v", err)
	}
	want := []string{
		`- button "Add to Cart" [ref=e12]`,
		`- button "Add to Cart" [ref=e13]`,
	}
	if !reflect.DeepEqual(lines, want) {
		t.Fatalf("lines = %#v, want %#v", lines, want)
	}
}

func TestGrepMatchLinesRequiresStructuredMatches(t *testing.T) {
	_, err := grepMatchLines(&mcp.ToolResult{
		Content: []mcp.ContentItem{{Type: "text", Text: `- button "Buy" [ref=e1]`}},
	})
	if err == nil {
		t.Fatal("grepMatchLines() error = nil, want missing structured matches error")
	}
	if !strings.Contains(err.Error(), "structured matches") {
		t.Fatalf("error = %q, want structured matches message", err.Error())
	}
}

func TestFindNoMatchStopsBeforeAct(t *testing.T) {
	query := findQuery{mode: "text", text: "missing", nth: 1}
	if _, err := selectFindMatch(findMatches([]string{`- button "Buy" [ref=e1]`}, query), query); err == nil {
		t.Fatal("selectFindMatch() error = nil, want no-match error")
	}
}

func TestFindRejectsInvalidNth(t *testing.T) {
	query := findQuery{mode: "text", text: "Buy", nth: -1}
	if _, err := selectFindMatch([]findMatch{{ref: "e1", line: `- button "Buy" [ref=e1]`}}, query); err == nil {
		t.Fatal("selectFindMatch() error = nil, want invalid nth error")
	}
}

func TestFindGrepToolArgsUsesBoundedDefault(t *testing.T) {
	got := findGrepToolArgs(7, findQuery{mode: "text", text: "Buy"})
	want := map[string]any{
		"page":    7,
		"pattern": "Buy",
		"over":    "ax",
		"limit":   findDefaultGrepLimit,
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("find grep args = %#v, want %#v", got, want)
	}
}

func TestFindGrepToolArgsExpandsToNth(t *testing.T) {
	got := findGrepToolArgs(7, findQuery{mode: "text", text: "Buy", nth: 150, limit: 10})
	if got["limit"] != 150 {
		t.Fatalf("limit = %v, want nth-sized search", got["limit"])
	}
}

func TestFindActionCalls(t *testing.T) {
	match := findMatch{ref: "e12", line: `- textbox "Search" [ref=e12]`}
	tests := []struct {
		name   string
		action findAction
		want   []toolCall
	}{
		{
			name:   "fill",
			action: findAction{kind: "fill", value: "hello"},
			want: []toolCall{{name: "act", args: map[string]any{
				"page":  7,
				"kind":  "fill",
				"ref":   "e12",
				"value": "hello",
				"clear": true,
			}}},
		},
		{
			name:   "type",
			action: findAction{kind: "type", value: "hello"},
			want: []toolCall{
				{name: "act", args: map[string]any{"page": 7, "kind": "focus", "ref": "e12"}},
				{name: "act", args: map[string]any{"page": 7, "kind": "type", "text": "hello"}},
			},
		},
		{
			name:   "select",
			action: findAction{kind: "select", value: "Large"},
			want: []toolCall{{name: "act", args: map[string]any{
				"page":  7,
				"kind":  "select",
				"ref":   "e12",
				"value": "Large",
			}}},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := findActionCalls(7, match, tt.action)
			if err != nil {
				t.Fatalf("findActionCalls() error = %v", err)
			}
			if !reflect.DeepEqual(got, tt.want) {
				t.Fatalf("calls = %#v, want %#v", got, tt.want)
			}
		})
	}
}

func TestTabsListResultUsesCanonicalPageField(t *testing.T) {
	result := tabsListResult(&mcp.ToolResult{
		StructuredContent: map[string]any{
			"pages": []any{
				map[string]any{"pageId": 42, "url": "https://example.com", "title": "Example"},
			},
		},
	})

	if got := result.StructuredContent["count"]; got != 1 {
		t.Fatalf("count = %v, want 1", got)
	}
	pages, ok := result.StructuredContent["pages"].([]any)
	if !ok || len(pages) != 1 {
		t.Fatalf("pages = %#v, want one page", result.StructuredContent["pages"])
	}
	page, ok := pages[0].(map[string]any)
	if !ok {
		t.Fatalf("page = %#v, want map", pages[0])
	}
	if _, exists := page["pageId"]; exists {
		t.Fatalf("page includes legacy pageId: %#v", page)
	}
	if got := numberValue(page["page"]); got != 42 {
		t.Fatalf("page = %d, want 42", got)
	}
}

func TestOpenResultUsesCanonicalPageField(t *testing.T) {
	result := openResult("https://example.com", &mcp.ToolResult{
		Content: []mcp.ContentItem{{Type: "text", Text: "opened page 42"}},
		StructuredContent: map[string]any{
			"pageId": 42,
		},
	})

	if _, exists := result.StructuredContent["pageId"]; exists {
		t.Fatalf("open result includes legacy pageId: %#v", result.StructuredContent)
	}
	if got := numberValue(result.StructuredContent["page"]); got != 42 {
		t.Fatalf("page = %d, want 42", got)
	}
	if got := result.TextContent(); got != "page=42\nurl=https://example.com" {
		t.Fatalf("open text = %q, want stable page/url lines", got)
	}
}

func TestActivePageResultUsesCanonicalPageField(t *testing.T) {
	result := activePageResult(&mcp.ToolResult{
		StructuredContent: map[string]any{
			"action": "active",
			"page": map[string]any{
				"pageId":   42,
				"tabId":    9,
				"title":    "Example",
				"url":      "https://example.com",
				"isActive": true,
			},
		},
	})

	if _, exists := result.StructuredContent["pageId"]; exists {
		t.Fatalf("active result includes legacy pageId: %#v", result.StructuredContent)
	}
	if nested, ok := result.StructuredContent["page"].(map[string]any); ok {
		t.Fatalf("active result nested page object: %#v", nested)
	}
	if got := numberValue(result.StructuredContent["page"]); got != 42 {
		t.Fatalf("page = %d, want 42", got)
	}
	if got := numberValue(result.StructuredContent["tabId"]); got != 9 {
		t.Fatalf("tabId = %d, want 9", got)
	}
}

func TestFillToolArgsFromNoClearFlag(t *testing.T) {
	cmd := &cobra.Command{}
	cmd.Flags().Bool("no-clear", false, "")
	if err := cmd.Flags().Parse([]string{"--no-clear"}); err != nil {
		t.Fatalf("parse --no-clear: %v", err)
	}

	got := fillToolArgsFromCommand(cmd, 7, "e12", "hello")
	want := map[string]any{
		"page":  7,
		"kind":  "fill",
		"ref":   "e12",
		"value": "hello",
		"clear": false,
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("fill args = %#v, want %#v", got, want)
	}
}

func TestOpenInWindowCodeUsesCompactRunBridge(t *testing.T) {
	code := openInWindowCode("https://example.com/?q=one two", true, false, 9)
	for _, want := range []string{
		"browser.pages.newPage",
		`"https://example.com/?q=one two"`,
		"hidden: true",
		"background: false",
		"windowId: 9",
	} {
		if !strings.Contains(code, want) {
			t.Fatalf("openInWindowCode() missing %q in:\n%s", want, code)
		}
	}
}

func TestScreenshotToolArgsRejectsUnsupportedQualityFormat(t *testing.T) {
	_, err := screenshotToolArgs(7, "webp", false, 20, true)
	if err == nil {
		t.Fatal("screenshotToolArgs() error = nil, want unsupported quality error")
	}

	got, err := screenshotToolArgs(7, "jpeg", true, 80, true)
	if err != nil {
		t.Fatalf("screenshotToolArgs() error = %v", err)
	}
	want := map[string]any{
		"page":     7,
		"format":   "jpeg",
		"fullPage": true,
		"quality":  80,
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("screenshot args = %#v, want %#v", got, want)
	}
}

func TestWriteScreenshotUsesStructuredImage(t *testing.T) {
	dir := t.TempDir()
	filename := filepath.Join(dir, "shot.png")
	result := &mcp.ToolResult{
		Content: []mcp.ContentItem{{Type: "image", Data: base64.StdEncoding.EncodeToString([]byte("legacy"))}},
		StructuredContent: map[string]any{
			"page":   7,
			"format": "png",
			"image":  base64.StdEncoding.EncodeToString([]byte("structured")),
		},
	}

	if err := writeScreenshot(result, filename); err != nil {
		t.Fatalf("writeScreenshot() error = %v", err)
	}
	data, err := os.ReadFile(filename)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "structured" {
		t.Fatalf("written data = %q, want structured image data", data)
	}

	addScreenshotPath(result, filename)
	if result.StructuredContent["page"] != 7 {
		t.Fatalf("page metadata = %v, want preserved", result.StructuredContent["page"])
	}
	if result.StructuredContent["path"] != filename {
		t.Fatalf("path metadata = %v, want %q", result.StructuredContent["path"], filename)
	}
}

func TestScreenshotImageDataRequiresStructuredImage(t *testing.T) {
	_, err := screenshotImageData(&mcp.ToolResult{
		Content: []mcp.ContentItem{{Type: "image", Data: base64.StdEncoding.EncodeToString([]byte("legacy"))}},
	})
	if err == nil {
		t.Fatal("screenshotImageData() error = nil, want missing structured image error")
	}
	if !strings.Contains(err.Error(), "structured image") {
		t.Fatalf("error = %q, want structured image message", err.Error())
	}
}

func TestCopyDownloadFileRejectsUnsafeNames(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "source")
	if err := os.WriteFile(src, []byte("data"), 0644); err != nil {
		t.Fatal(err)
	}

	for _, filename := range []string{"", ".", "..", "../report.csv", "nested/report.csv", `nested\report.csv`, "/tmp/report.csv"} {
		t.Run(filename, func(t *testing.T) {
			if _, err := copyDownloadFile(src, dir, filename); err == nil {
				t.Fatalf("copyDownloadFile(%q) error = nil, want unsafe filename error", filename)
			}
		})
	}
}

func TestCopyDownloadFileAvoidsOverwrite(t *testing.T) {
	dir := t.TempDir()
	src := filepath.Join(dir, "source")
	if err := os.WriteFile(src, []byte("new"), 0644); err != nil {
		t.Fatal(err)
	}
	existing := filepath.Join(dir, "report.csv")
	if err := os.WriteFile(existing, []byte("old"), 0644); err != nil {
		t.Fatal(err)
	}

	dst, err := copyDownloadFile(src, dir, "report.csv")
	if err != nil {
		t.Fatalf("copyDownloadFile() error = %v", err)
	}
	if filepath.Base(dst) != "report-1.csv" {
		t.Fatalf("destination = %q, want report-1.csv suffix", dst)
	}
	existingData, err := os.ReadFile(existing)
	if err != nil {
		t.Fatal(err)
	}
	if string(existingData) != "old" {
		t.Fatalf("existing file = %q, want old", existingData)
	}
	newData, err := os.ReadFile(dst)
	if err != nil {
		t.Fatal(err)
	}
	if string(newData) != "new" {
		t.Fatalf("copied file = %q, want new", newData)
	}
}
