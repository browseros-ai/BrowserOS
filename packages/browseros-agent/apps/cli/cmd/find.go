package cmd

import (
	"fmt"
	"regexp"
	"strconv"
	"strings"

	"browseros-cli/mcp"
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

type findQuery struct {
	mode  string
	text  string
	role  string
	name  string
	nth   int
	limit int
}

type findAction struct {
	kind  string
	value string
}

type findMatch struct {
	ref  string
	line string
}

type toolCall struct {
	name string
	args map[string]any
}

const findDefaultGrepLimit = 100

func init() {
	cmd := &cobra.Command{
		Use:         "find text <text> <action> | find role <role> <action>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Find an element in the snapshot and act on it",
		Args:        cobra.MinimumNArgs(3),
		Run: func(cmd *cobra.Command, args []string) {
			query, action, err := parseFindArgs(cmd, args)
			if err != nil {
				output.Error(err.Error(), 3)
			}
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()

			grepResult, err := c.CallTool("grep", findGrepToolArgs(pageID, query))
			if err != nil {
				output.Error(err.Error(), 1)
			}
			lines, err := grepMatchLines(grepResult)
			if err != nil {
				output.Error(err.Error(), 1)
			}
			matches := findMatches(lines, query)
			selected, err := selectFindMatch(matches, query)
			if err != nil {
				output.Error(err.Error(), 1)
			}
			calls, err := findActionCalls(pageID, selected, action)
			if err != nil {
				output.Error(err.Error(), 3)
			}

			result, err := callTools(c, calls)
			if err != nil {
				output.Error(err.Error(), 1)
			}
			report := findReport(pageID, matches, selected, query, action, result)
			if jsonOut {
				output.JSONRaw(report.StructuredContent)
			} else {
				output.Confirm(report.TextContent())
			}
		},
	}
	cmd.Flags().String("name", "", "Accessible name substring for role matches")
	cmd.Flags().Int("nth", 1, "One-based match index to select")
	cmd.Flags().Int("limit", findDefaultGrepLimit, "Maximum snapshot candidates to search")
	rootCmd.AddCommand(cmd)
}

func parseFindArgs(cmd *cobra.Command, args []string) (findQuery, findAction, error) {
	nth, _ := cmd.Flags().GetInt("nth")
	if nth < 1 {
		return findQuery{}, findAction{}, fmt.Errorf("--nth must be 1 or greater")
	}
	limit, _ := cmd.Flags().GetInt("limit")
	if limit < 1 {
		return findQuery{}, findAction{}, fmt.Errorf("--limit must be 1 or greater")
	}
	mode := args[0]
	query := findQuery{mode: mode, nth: nth, limit: limit}
	var actionArgs []string
	switch mode {
	case "text":
		query.text = args[1]
		actionArgs = args[2:]
	case "role":
		query.role = args[1]
		query.name, _ = cmd.Flags().GetString("name")
		actionArgs = args[2:]
	default:
		return findQuery{}, findAction{}, fmt.Errorf("find mode must be text or role")
	}
	action, err := parseFindAction(actionArgs)
	return query, action, err
}

func parseFindAction(args []string) (findAction, error) {
	if len(args) == 0 {
		return findAction{}, fmt.Errorf("find action is required")
	}
	action := findAction{kind: args[0]}
	switch action.kind {
	case "click", "hover", "check", "uncheck", "focus":
		if len(args) != 1 {
			return findAction{}, fmt.Errorf("%s does not take a value", action.kind)
		}
	case "fill", "type", "select":
		if len(args) < 2 {
			return findAction{}, fmt.Errorf("%s requires a value", action.kind)
		}
		action.value = strings.Join(args[1:], " ")
	default:
		return findAction{}, fmt.Errorf("unsupported find action: %s", action.kind)
	}
	return action, nil
}

func (q findQuery) grepPattern() string {
	if q.mode == "role" {
		if q.name != "" {
			return regexp.QuoteMeta(q.name)
		}
		return regexp.QuoteMeta(q.role)
	}
	return regexp.QuoteMeta(q.text)
}

func findGrepToolArgs(pageID int, query findQuery) map[string]any {
	return grepToolArgs(pageID, query.grepPattern(), "ax", query.grepLimit())
}

func (q findQuery) grepLimit() int {
	limit := q.limit
	if limit < 1 {
		limit = findDefaultGrepLimit
	}
	if q.nth > limit {
		return q.nth
	}
	return limit
}

// grepMatchLines extracts the server-provided candidate lines that find acts on.
func grepMatchLines(result *mcp.ToolResult) ([]string, error) {
	if result == nil || result.StructuredContent == nil {
		return nil, fmt.Errorf("grep response missing structured matches")
	}
	raw, ok := result.StructuredContent["matches"]
	if !ok {
		return nil, fmt.Errorf("grep response missing structured matches")
	}
	switch matches := raw.(type) {
	case []any:
		lines := make([]string, 0, len(matches))
		for _, match := range matches {
			line, ok := match.(string)
			if !ok {
				return nil, fmt.Errorf("grep response included a non-string match")
			}
			lines = append(lines, line)
		}
		return lines, nil
	case []string:
		return append([]string(nil), matches...), nil
	default:
		return nil, fmt.Errorf("grep response missing structured matches")
	}
}

func findMatches(lines []string, query findQuery) []findMatch {
	matches := []findMatch{}
	for _, line := range lines {
		ref, ok := findLineRef(line)
		if !ok || !findLineMatches(line, query) {
			continue
		}
		matches = append(matches, findMatch{ref: ref, line: line})
	}
	return matches
}

func findLineRef(line string) (string, bool) {
	match := snapshotRefPattern.FindStringSubmatch(line)
	if len(match) != 2 {
		return "", false
	}
	return "e" + match[1], true
}

func findLineMatches(line string, query findQuery) bool {
	lower := strings.ToLower(line)
	switch query.mode {
	case "text":
		return strings.Contains(lower, strings.ToLower(query.text))
	case "role":
		if query.role != "" && lineRole(line) != strings.ToLower(query.role) {
			return false
		}
		return query.name == "" || strings.Contains(lower, strings.ToLower(query.name))
	default:
		return false
	}
}

func lineRole(line string) string {
	trimmed := strings.TrimSpace(line)
	trimmed = strings.TrimPrefix(trimmed, "-")
	trimmed = strings.TrimSpace(trimmed)
	fields := strings.Fields(trimmed)
	if len(fields) == 0 {
		return ""
	}
	return strings.ToLower(fields[0])
}

func selectFindMatch(matches []findMatch, query findQuery) (findMatch, error) {
	if len(matches) == 0 {
		return findMatch{}, fmt.Errorf("find: no matches")
	}
	nth := query.nth
	if nth < 1 {
		return findMatch{}, fmt.Errorf("find: --nth must be 1 or greater")
	}
	if nth > len(matches) {
		return findMatch{}, fmt.Errorf("find: only %d matches, --nth %d is out of range", len(matches), nth)
	}
	return matches[nth-1], nil
}

// findActionCalls maps a selected snapshot ref to the act calls needed for the requested action.
func findActionCalls(pageID int, match findMatch, action findAction) ([]toolCall, error) {
	ref := match.ref
	switch action.kind {
	case "click", "hover", "check", "uncheck", "focus":
		return []toolCall{{name: "act", args: map[string]any{
			"page": pageID,
			"kind": action.kind,
			"ref":  ref,
		}}}, nil
	case "fill":
		return []toolCall{{name: "act", args: map[string]any{
			"page":  pageID,
			"kind":  "fill",
			"ref":   ref,
			"value": action.value,
			"clear": true,
		}}}, nil
	case "type":
		return []toolCall{
			{name: "act", args: map[string]any{"page": pageID, "kind": "focus", "ref": ref}},
			{name: "act", args: map[string]any{"page": pageID, "kind": "type", "text": action.value}},
		}, nil
	case "select":
		return []toolCall{{name: "act", args: map[string]any{
			"page":  pageID,
			"kind":  "select",
			"ref":   ref,
			"value": action.value,
		}}}, nil
	default:
		return nil, fmt.Errorf("unsupported find action: %s", action.kind)
	}
}

func findReport(pageID int, matches []findMatch, selected findMatch, query findQuery, action findAction, result *mcp.ToolResult) *mcp.ToolResult {
	selectedIndex := 1
	for i, match := range matches {
		if match.ref == selected.ref && match.line == selected.line {
			selectedIndex = i + 1
			break
		}
	}
	limit := query.grepLimit()
	limitReached := len(matches) >= limit
	countLabel := strconv.Itoa(len(matches))
	if limitReached {
		countLabel += "+"
	}
	ref := "@" + selected.ref
	text := fmt.Sprintf("match %d/%s %s\n%s", selectedIndex, countLabel, ref, displayElementRefs(selected.line))
	if result != nil && result.TextContent() != "" {
		text += "\n" + result.TextContent()
	}
	data := map[string]any{
		"page":          pageID,
		"ref":           ref,
		"line":          displayElementRefs(selected.line),
		"matchCount":    len(matches),
		"selectedIndex": selectedIndex,
		"action":        action.kind,
		"matchLimit":    limit,
	}
	if limitReached {
		data["matchLimitReached"] = true
	}
	if action.value != "" {
		data["value"] = action.value
	}
	if result != nil {
		data["result"] = result.StructuredContent
	}
	return textResult(text, data)
}
