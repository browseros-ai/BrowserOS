package output

import (
	"encoding/json"
	"fmt"
	"os"

	"browseros-cli/mcp"

	"github.com/fatih/color"
)

var (
	errColor  = color.New(color.FgRed, color.Bold)
	dimColor  = color.New(color.Faint)
	boldColor = color.New(color.Bold)
)

// JSON outputs raw JSON to stdout. If structuredContent exists, use it;
// otherwise fall back to serializing the full result.
func JSON(result *mcp.ToolResult) {
	if result.StructuredContent != nil {
		data, _ := json.Marshal(result.StructuredContent)
		fmt.Println(string(data))
		return
	}
	data, _ := json.Marshal(result)
	fmt.Println(string(data))
}

// JSONRaw outputs any value as JSON.
func JSONRaw(v any) {
	data, _ := json.Marshal(v)
	fmt.Println(string(data))
}

// Text prints a text result to stdout.
func Text(result *mcp.ToolResult) {
	fmt.Println(result.TextContent())
}

// Confirm prints a short confirmation message.
func Confirm(msg string) {
	fmt.Println(msg)
}

// Error prints an error to stderr and exits with the given code.
func Error(msg string, code int) {
	errColor.Fprintf(os.Stderr, "Error: %s\n", msg)
	os.Exit(code)
}

// Errorf formats and prints an error to stderr and exits.
func Errorf(code int, format string, args ...any) {
	errColor.Fprintf(os.Stderr, "Error: "+format+"\n", args...)
	os.Exit(code)
}

// PageList formats a list of pages for human display.
func PageList(result *mcp.ToolResult) {
	if result.StructuredContent == nil {
		Text(result)
		return
	}

	pages, ok := result.StructuredContent["pages"].([]any)
	if !ok {
		Text(result)
		return
	}

	if len(pages) == 0 {
		fmt.Println("No pages open.")
		return
	}

	for _, p := range pages {
		page, ok := p.(map[string]any)
		if !ok {
			continue
		}
		pageID := intVal(page["pageId"])
		if pageID == 0 {
			pageID = intVal(page["page"])
		}
		tabID := intVal(page["tabId"])
		title := strVal(page["title"])
		url := strVal(page["url"])
		active := boolVal(page["isActive"])

		marker := ""
		if active {
			marker = " " + boldColor.Sprint("[ACTIVE]")
		}

		if tabID == 0 {
			fmt.Printf("  %d. %s%s\n", pageID, title, marker)
		} else {
			fmt.Printf("  %d. %s (tab %d)%s\n", pageID, title, tabID, marker)
		}
		fmt.Printf("     %s\n", dimColor.Sprint(url))
	}
}

// ActivePage formats a single active page for human display.
func ActivePage(result *mcp.ToolResult) {
	if result.StructuredContent == nil {
		Text(result)
		return
	}

	sc := result.StructuredContent
	page := sc
	if nested, ok := sc["page"].(map[string]any); ok {
		page = nested
	}

	pageID := intVal(page["pageId"])
	if pageID == 0 {
		pageID = intVal(page["page"])
	}
	tabID := intVal(page["tabId"])
	title := strVal(page["title"])
	url := strVal(page["url"])

	if tabID == 0 {
		fmt.Printf("Active page: %d\n", pageID)
	} else {
		fmt.Printf("Active page: %d (tab %d)\n", pageID, tabID)
	}
	fmt.Println(title)
	fmt.Println(dimColor.Sprint(url))
}

func intVal(v any) int {
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
	}
	return 0
}

func strVal(v any) string {
	if s, ok := v.(string); ok {
		return s
	}
	return ""
}

func boolVal(v any) bool {
	if b, ok := v.(bool); ok {
		return b
	}
	return false
}
