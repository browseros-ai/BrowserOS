package cmd

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"

	"browseros-cli/mcp"
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

type toolCaller interface {
	CallTool(name string, args map[string]any) (*mcp.ToolResult, error)
}

type batchOptions struct {
	page    int
	pageSet bool
	bail    bool
}

type batchResult struct {
	Index      int            `json:"index"`
	Command    string         `json:"command"`
	OK         bool           `json:"ok"`
	Text       string         `json:"text,omitempty"`
	Error      string         `json:"error,omitempty"`
	Structured map[string]any `json:"structured,omitempty"`
}

func init() {
	cmd := &cobra.Command{
		Use:         "batch [command...]",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Run browser commands sequentially with one MCP session",
		Args:        cobra.ArbitraryArgs,
		Run: func(cmd *cobra.Command, args []string) {
			bail, _ := cmd.Flags().GetBool("bail")
			commands, err := batchCommands(args, os.Stdin)
			if err != nil {
				output.Error(err.Error(), 3)
			}
			opts := batchOptions{
				page:    pageFlag,
				pageSet: rootCmd.PersistentFlags().Changed("page"),
				bail:    bail,
			}
			if results := preflightBatchCommands(commands, opts); len(results) > 0 {
				writeBatchResults(results)
				os.Exit(1)
			}

			c := newClient()
			var results []batchResult
			if err := c.WithSession(func(shared *mcp.Client) error {
				results = runBatchCommands(shared, commands, opts)
				return nil
			}); err != nil {
				output.Error(err.Error(), 1)
			}

			writeBatchResults(results)
			if failedBatch(results) {
				os.Exit(1)
			}
		},
	}
	cmd.Flags().Bool("bail", false, "Stop after the first failed subcommand")
	rootCmd.AddCommand(cmd)
}

func writeBatchResults(results []batchResult) {
	if jsonOut {
		output.JSONRaw(results)
	} else {
		output.Confirm(formatBatchResults(results))
	}
}

func batchCommands(args []string, stdin io.Reader) ([]string, error) {
	if len(args) > 0 {
		return args, nil
	}
	if file, ok := stdin.(*os.File); ok {
		info, err := file.Stat()
		if err == nil && info.Mode()&os.ModeCharDevice != 0 {
			return nil, fmt.Errorf("batch requires command strings or stdin lines")
		}
	}
	scanner := bufio.NewScanner(stdin)
	commands := []string{}
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line != "" {
			commands = append(commands, line)
		}
	}
	if err := scanner.Err(); err != nil {
		return nil, err
	}
	if len(commands) == 0 {
		return nil, fmt.Errorf("batch requires command strings or stdin lines")
	}
	return commands, nil
}

// preflightBatchCommands validates command syntax and page routing before a shared MCP session can mutate pages.
func preflightBatchCommands(commands []string, opts batchOptions) []batchResult {
	results := make([]batchResult, 0)
	for i, raw := range commands {
		if err := validateBatchCommand(raw, opts); err != nil {
			results = append(results, batchResult{Index: i + 1, Command: raw, OK: false, Error: err.Error()})
			if opts.bail {
				break
			}
		}
	}
	return results
}

func validateBatchCommand(raw string, opts batchOptions) error {
	tokens, err := splitBatchCommand(raw)
	if err != nil {
		return err
	}
	_, remaining, err := batchPage(tokens, opts)
	if err != nil {
		return err
	}
	if len(remaining) == 0 {
		return fmt.Errorf("empty batch command")
	}

	switch remaining[0] {
	case "nav":
		if len(remaining) != 2 {
			return fmt.Errorf("nav requires one url")
		}
	case "back", "forward", "reload":
		if len(remaining) != 1 {
			return fmt.Errorf("%s does not take arguments", remaining[0])
		}
	case "eval":
		if len(remaining) < 2 {
			return fmt.Errorf("eval requires code")
		}
	case "press", "key":
		if len(remaining) != 2 {
			return fmt.Errorf("%s requires one key", remaining[0])
		}
	case "type":
		if len(remaining) < 2 {
			return fmt.Errorf("type requires text")
		}
	case "click":
		if len(remaining) != 2 {
			return fmt.Errorf("click requires one ref")
		}
		_, err = elementRef(remaining[1])
	case "hover", "focus", "check", "uncheck":
		if len(remaining) != 2 {
			return fmt.Errorf("%s requires one ref", remaining[0])
		}
		_, err = elementRef(remaining[1])
	case "fill":
		if len(remaining) < 3 {
			return fmt.Errorf("fill requires a ref and value")
		}
		_, err = elementRef(remaining[1])
	case "select":
		if len(remaining) < 3 {
			return fmt.Errorf("select requires a ref and value")
		}
		_, err = elementRef(remaining[1])
	case "snapshot", "snap":
		if len(remaining) != 1 {
			return fmt.Errorf("%s does not take arguments", remaining[0])
		}
	case "read", "text", "links":
		_, err = batchReadOptions(remaining)
	case "grep":
		_, _, _, err = batchGrepArgs(remaining)
	case "find":
		_, _, err = parseFindTokens(remaining[1:])
	default:
		return fmt.Errorf("unsupported batch command: %s", remaining[0])
	}
	return err
}

// runBatchCommands executes each command string independently so failures can be reported per step.
func runBatchCommands(c toolCaller, commands []string, opts batchOptions) []batchResult {
	results := make([]batchResult, 0, len(commands))
	for i, raw := range commands {
		result := batchResult{Index: i + 1, Command: raw}
		toolResult, err := runBatchCommand(c, raw, opts)
		if err != nil {
			result.OK = false
			result.Error = err.Error()
			results = append(results, result)
			if opts.bail {
				break
			}
			continue
		}
		result.OK = true
		result.Text = toolResult.TextContent()
		result.Structured = toolResult.StructuredContent
		results = append(results, result)
	}
	return results
}

// runBatchCommand maps one shell-like command string onto the existing compact browser tools.
func runBatchCommand(c toolCaller, raw string, opts batchOptions) (*mcp.ToolResult, error) {
	tokens, err := splitBatchCommand(raw)
	if err != nil {
		return nil, err
	}
	page, remaining, err := batchPage(tokens, opts)
	if err != nil {
		return nil, err
	}
	if len(remaining) == 0 {
		return nil, fmt.Errorf("empty batch command")
	}

	switch remaining[0] {
	case "nav":
		if len(remaining) != 2 {
			return nil, fmt.Errorf("nav requires one url")
		}
		return c.CallTool("navigate", map[string]any{"page": page, "action": "url", "url": remaining[1]})
	case "back", "forward", "reload":
		if len(remaining) != 1 {
			return nil, fmt.Errorf("%s does not take arguments", remaining[0])
		}
		return c.CallTool("navigate", map[string]any{"page": page, "action": remaining[0]})
	case "eval":
		if len(remaining) < 2 {
			return nil, fmt.Errorf("eval requires code")
		}
		return c.CallTool("evaluate", map[string]any{"page": page, "code": evalCode(strings.Join(remaining[1:], " "))})
	case "press", "key":
		if len(remaining) != 2 {
			return nil, fmt.Errorf("%s requires one key", remaining[0])
		}
		return c.CallTool("act", pressToolArgs(page, remaining[1]))
	case "type":
		if len(remaining) < 2 {
			return nil, fmt.Errorf("type requires text")
		}
		return c.CallTool("act", typeToolArgs(page, strings.Join(remaining[1:], " ")))
	case "click":
		if len(remaining) != 2 {
			return nil, fmt.Errorf("click requires one ref")
		}
		ref, err := elementRef(remaining[1])
		if err != nil {
			return nil, err
		}
		return c.CallTool("act", clickToolArgs(page, ref, "left", 1))
	case "hover", "focus", "check", "uncheck":
		if len(remaining) != 2 {
			return nil, fmt.Errorf("%s requires one ref", remaining[0])
		}
		ref, err := elementRef(remaining[1])
		if err != nil {
			return nil, err
		}
		return c.CallTool("act", map[string]any{"page": page, "kind": remaining[0], "ref": ref})
	case "fill":
		if len(remaining) < 3 {
			return nil, fmt.Errorf("fill requires a ref and value")
		}
		ref, err := elementRef(remaining[1])
		if err != nil {
			return nil, err
		}
		return c.CallTool("act", fillToolArgs(page, ref, strings.Join(remaining[2:], " "), true))
	case "select":
		if len(remaining) < 3 {
			return nil, fmt.Errorf("select requires a ref and value")
		}
		ref, err := elementRef(remaining[1])
		if err != nil {
			return nil, err
		}
		return c.CallTool("act", map[string]any{"page": page, "kind": "select", "ref": ref, "value": strings.Join(remaining[2:], " ")})
	case "snapshot", "snap":
		if len(remaining) != 1 {
			return nil, fmt.Errorf("%s does not take arguments", remaining[0])
		}
		return c.CallTool("snapshot", map[string]any{"page": page})
	case "read", "text", "links":
		return runBatchRead(c, page, remaining)
	case "grep":
		pattern, over, limit, err := batchGrepArgs(remaining)
		if err != nil {
			return nil, err
		}
		return c.CallTool("grep", grepToolArgs(page, pattern, over, limit))
	case "find":
		return runBatchFind(c, page, remaining[1:])
	default:
		return nil, fmt.Errorf("unsupported batch command: %s", remaining[0])
	}
}

// batchPage applies an optional subcommand page override over the outer batch page.
func batchPage(tokens []string, opts batchOptions) (int, []string, error) {
	page := opts.page
	pageSet := opts.pageSet
	remaining := make([]string, 0, len(tokens))
	for i := 0; i < len(tokens); i++ {
		token := tokens[i]
		switch {
		case token == "-p" || token == "--page":
			if i+1 >= len(tokens) {
				return 0, nil, fmt.Errorf("%s requires a page id", token)
			}
			value, err := strconv.Atoi(tokens[i+1])
			if err != nil {
				return 0, nil, fmt.Errorf("invalid page id: %s", tokens[i+1])
			}
			if err := validatePageID(value); err != nil {
				return 0, nil, err
			}
			page = value
			pageSet = true
			i++
		case strings.HasPrefix(token, "-p="):
			value, err := strconv.Atoi(strings.TrimPrefix(token, "-p="))
			if err != nil {
				return 0, nil, fmt.Errorf("invalid page id: %s", strings.TrimPrefix(token, "-p="))
			}
			if err := validatePageID(value); err != nil {
				return 0, nil, err
			}
			page = value
			pageSet = true
		case strings.HasPrefix(token, "--page="):
			value, err := strconv.Atoi(strings.TrimPrefix(token, "--page="))
			if err != nil {
				return 0, nil, fmt.Errorf("invalid page id: %s", strings.TrimPrefix(token, "--page="))
			}
			if err := validatePageID(value); err != nil {
				return 0, nil, err
			}
			page = value
			pageSet = true
		default:
			remaining = append(remaining, token)
		}
	}
	if !pageSet {
		return 0, nil, fmt.Errorf("page id is required: pass -p/--page <id> to batch or the subcommand")
	}
	if err := validatePageID(page); err != nil {
		return 0, nil, err
	}
	return page, remaining, nil
}

func runBatchRead(c toolCaller, page int, tokens []string) (*mcp.ToolResult, error) {
	opts, err := batchReadOptions(tokens)
	if err != nil {
		return nil, err
	}
	return c.CallTool("read", readToolArgs(page, opts))
}

func batchReadOptions(tokens []string) (readOptions, error) {
	opts := readOptions{format: "markdown"}
	switch tokens[0] {
	case "text":
	case "links":
		opts.format = "links"
	case "read":
	default:
		return readOptions{}, fmt.Errorf("unsupported read command in batch: %s", tokens[0])
	}
	for i := 1; i < len(tokens); i++ {
		token := tokens[i]
		switch token {
		case "--md":
			opts.format = "markdown"
		case "--text":
			opts.format = "text"
		case "--links":
			if tokens[0] == "text" {
				opts.includeLinks = true
			} else {
				opts.format = "links"
			}
		case "--selector":
			if i+1 >= len(tokens) {
				return readOptions{}, fmt.Errorf("--selector requires a value")
			}
			opts.selector = tokens[i+1]
			i++
		case "--viewport":
			opts.viewportOnly = true
		case "--include-links":
			opts.includeLinks = true
		case "--images":
			opts.includeImages = true
		default:
			if strings.HasPrefix(token, "--selector=") {
				opts.selector = strings.TrimPrefix(token, "--selector=")
				continue
			}
			return readOptions{}, fmt.Errorf("unsupported read flag in batch: %s", token)
		}
	}
	if tokens[0] == "links" && opts.format != "links" {
		return readOptions{}, fmt.Errorf("links does not support format flags in batch")
	}
	return opts, nil
}

func batchGrepArgs(tokens []string) (string, string, int, error) {
	over := "ax"
	limit := 0
	patterns := []string{}
	for i := 1; i < len(tokens); i++ {
		token := tokens[i]
		switch {
		case token == "--content":
			over = "content"
		case token == "--limit":
			if i+1 >= len(tokens) {
				return "", "", 0, fmt.Errorf("--limit requires a value")
			}
			value, err := strconv.Atoi(tokens[i+1])
			if err != nil || value < 1 {
				return "", "", 0, fmt.Errorf("invalid grep limit: %s", tokens[i+1])
			}
			limit = value
			i++
		case strings.HasPrefix(token, "--limit="):
			value := strings.TrimPrefix(token, "--limit=")
			parsed, err := strconv.Atoi(value)
			if err != nil || parsed < 1 {
				return "", "", 0, fmt.Errorf("invalid grep limit: %s", value)
			}
			limit = parsed
		default:
			patterns = append(patterns, token)
		}
	}
	if len(patterns) != 1 {
		return "", "", 0, fmt.Errorf("grep requires one pattern")
	}
	return patterns[0], over, limit, nil
}

func runBatchFind(c toolCaller, page int, args []string) (*mcp.ToolResult, error) {
	query, action, err := parseFindTokens(args)
	if err != nil {
		return nil, err
	}
	grepResult, err := c.CallTool("grep", findGrepToolArgs(page, query))
	if err != nil {
		return nil, err
	}
	lines, err := grepMatchLines(grepResult)
	if err != nil {
		return nil, err
	}
	matches := findMatches(lines, query)
	selected, err := selectFindMatch(matches, query)
	if err != nil {
		return nil, err
	}
	calls, err := findActionCalls(page, selected, action)
	if err != nil {
		return nil, err
	}
	result, err := callTools(c, calls)
	if err != nil {
		return nil, err
	}
	return findReport(page, matches, selected, query, action, result), nil
}

func parseFindTokens(args []string) (findQuery, findAction, error) {
	filtered := make([]string, 0, len(args))
	name := ""
	nth := 1
	limit := findDefaultGrepLimit
	for i := 0; i < len(args); i++ {
		token := args[i]
		switch {
		case token == "--name":
			if i+1 >= len(args) {
				return findQuery{}, findAction{}, fmt.Errorf("--name requires a value")
			}
			name = args[i+1]
			i++
		case strings.HasPrefix(token, "--name="):
			name = strings.TrimPrefix(token, "--name=")
		case token == "--nth":
			if i+1 >= len(args) {
				return findQuery{}, findAction{}, fmt.Errorf("--nth requires a value")
			}
			value, err := strconv.Atoi(args[i+1])
			if err != nil {
				return findQuery{}, findAction{}, fmt.Errorf("invalid --nth: %s", args[i+1])
			}
			nth = value
			i++
		case strings.HasPrefix(token, "--nth="):
			value, err := strconv.Atoi(strings.TrimPrefix(token, "--nth="))
			if err != nil {
				return findQuery{}, findAction{}, fmt.Errorf("invalid --nth: %s", strings.TrimPrefix(token, "--nth="))
			}
			nth = value
		case token == "--limit":
			if i+1 >= len(args) {
				return findQuery{}, findAction{}, fmt.Errorf("--limit requires a value")
			}
			value, err := strconv.Atoi(args[i+1])
			if err != nil {
				return findQuery{}, findAction{}, fmt.Errorf("invalid --limit: %s", args[i+1])
			}
			limit = value
			i++
		case strings.HasPrefix(token, "--limit="):
			value, err := strconv.Atoi(strings.TrimPrefix(token, "--limit="))
			if err != nil {
				return findQuery{}, findAction{}, fmt.Errorf("invalid --limit: %s", strings.TrimPrefix(token, "--limit="))
			}
			limit = value
		default:
			filtered = append(filtered, token)
		}
	}
	if len(filtered) < 3 {
		return findQuery{}, findAction{}, fmt.Errorf("find requires a mode, query, and action")
	}
	if nth < 1 {
		return findQuery{}, findAction{}, fmt.Errorf("--nth must be 1 or greater")
	}
	if limit < 1 {
		return findQuery{}, findAction{}, fmt.Errorf("--limit must be 1 or greater")
	}
	query := findQuery{mode: filtered[0], nth: nth, limit: limit}
	var actionArgs []string
	switch query.mode {
	case "text":
		query.text = filtered[1]
		actionArgs = filtered[2:]
	case "role":
		query.role = filtered[1]
		query.name = name
		actionArgs = filtered[2:]
	default:
		return findQuery{}, findAction{}, fmt.Errorf("find mode must be text or role")
	}
	action, err := parseFindAction(actionArgs)
	return query, action, err
}

// splitBatchCommand handles the common quoted argument forms agents use in batch strings.
func splitBatchCommand(raw string) ([]string, error) {
	tokens := []string{}
	var current strings.Builder
	var quote rune
	tokenStarted := false
	runes := []rune(raw)
	for i := 0; i < len(runes); i++ {
		r := runes[i]
		if quote == '\'' {
			tokenStarted = true
			if r == quote {
				quote = 0
				continue
			}
			current.WriteRune(r)
			continue
		}
		if quote == '"' {
			tokenStarted = true
			if r == '\\' {
				if i+1 < len(runes) && (runes[i+1] == '"' || runes[i+1] == '\\') {
					i++
					current.WriteRune(runes[i])
					continue
				}
				current.WriteRune(r)
				continue
			}
			if r == quote {
				quote = 0
				continue
			}
			current.WriteRune(r)
			continue
		}
		if r == '\\' {
			tokenStarted = true
			if i+1 < len(runes) && (runes[i+1] == ' ' || runes[i+1] == '\t' || runes[i+1] == '\'' || runes[i+1] == '"' || runes[i+1] == '\\') {
				i++
				current.WriteRune(runes[i])
				continue
			}
			current.WriteRune(r)
			continue
		}
		if r == '\'' || r == '"' {
			quote = r
			tokenStarted = true
			continue
		}
		if r == ' ' || r == '\t' {
			if tokenStarted {
				tokens = append(tokens, current.String())
				current.Reset()
				tokenStarted = false
			}
			continue
		}
		current.WriteRune(r)
		tokenStarted = true
	}
	if quote != 0 {
		return nil, fmt.Errorf("unterminated quote in batch command")
	}
	if tokenStarted {
		tokens = append(tokens, current.String())
	}
	return tokens, nil
}

func callTools(c toolCaller, calls []toolCall) (*mcp.ToolResult, error) {
	var result *mcp.ToolResult
	for _, call := range calls {
		next, err := c.CallTool(call.name, call.args)
		if err != nil {
			return nil, err
		}
		result = next
	}
	return result, nil
}

func failedBatch(results []batchResult) bool {
	for _, result := range results {
		if !result.OK {
			return true
		}
	}
	return false
}

func formatBatchResults(results []batchResult) string {
	lines := make([]string, 0, len(results))
	for _, result := range results {
		status := "ok"
		body := result.Text
		if !result.OK {
			status = "failed"
			body = result.Error
		}
		lines = append(lines, fmt.Sprintf("[%d] %s: %s\n%s", result.Index, status, result.Command, body))
	}
	return strings.Join(lines, "\n\n")
}
