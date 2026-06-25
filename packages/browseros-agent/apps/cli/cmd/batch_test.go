package cmd

import (
	"errors"
	"reflect"
	"strings"
	"testing"

	"browseros-cli/mcp"
)

type fakeToolCaller struct {
	calls []toolCall
	fail  map[int]error
}

func (f *fakeToolCaller) CallTool(name string, args map[string]any) (*mcp.ToolResult, error) {
	f.calls = append(f.calls, toolCall{name: name, args: args})
	if err := f.fail[len(f.calls)]; err != nil {
		return nil, err
	}
	return textResult("ok", map[string]any{"ok": true}), nil
}

func TestBatchCommandsInheritAndOverridePage(t *testing.T) {
	caller := &fakeToolCaller{}
	results := runBatchCommands(caller, []string{"press Enter", "-p 9 press Escape", "nav https://example.com", "eval document.title", "hover @e3", "select @e4 Large"}, batchOptions{
		page:    7,
		pageSet: true,
		bail:    true,
	})

	if failedBatch(results) {
		t.Fatalf("batch failed: %#v", results)
	}
	want := []toolCall{
		{name: "act", args: map[string]any{"page": 7, "kind": "press", "key": "Enter"}},
		{name: "act", args: map[string]any{"page": 9, "kind": "press", "key": "Escape"}},
		{name: "navigate", args: map[string]any{"page": 7, "action": "url", "url": "https://example.com"}},
		{name: "evaluate", args: map[string]any{"page": 7, "code": evalCode("document.title")}},
		{name: "act", args: map[string]any{"page": 7, "kind": "hover", "ref": "e3"}},
		{name: "act", args: map[string]any{"page": 7, "kind": "select", "ref": "e4", "value": "Large"}},
	}
	if !reflect.DeepEqual(caller.calls, want) {
		t.Fatalf("calls = %#v, want %#v", caller.calls, want)
	}
}

func TestBatchBailStopsOnFirstFailure(t *testing.T) {
	caller := &fakeToolCaller{fail: map[int]error{1: errors.New("boom")}}
	results := runBatchCommands(caller, []string{"press Enter", "press Escape"}, batchOptions{
		page:    7,
		pageSet: true,
		bail:    true,
	})

	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	if !failedBatch(results) {
		t.Fatal("failedBatch() = false, want true")
	}
}

func TestBatchContinuesWithoutBail(t *testing.T) {
	caller := &fakeToolCaller{fail: map[int]error{1: errors.New("boom")}}
	results := runBatchCommands(caller, []string{"press Enter", "press Escape"}, batchOptions{
		page:    7,
		pageSet: true,
	})

	if len(results) != 2 {
		t.Fatalf("results = %d, want 2", len(results))
	}
	if !failedBatch(results) {
		t.Fatal("failedBatch() = false, want true")
	}
}

func TestBatchPreflightMissingPageBeforeSession(t *testing.T) {
	results := preflightBatchCommands([]string{"snapshot"}, batchOptions{})

	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	if results[0].OK {
		t.Fatal("preflight result OK = true, want false")
	}
	if !strings.Contains(results[0].Error, "page id is required") {
		t.Fatalf("error = %q, want missing page error", results[0].Error)
	}
}

func TestBatchPreflightRejectsInvalidFindNth(t *testing.T) {
	results := preflightBatchCommands([]string{"find --nth -1 text Buy click"}, batchOptions{
		page:    7,
		pageSet: true,
	})

	if len(results) != 1 {
		t.Fatalf("results = %d, want 1", len(results))
	}
	if !strings.Contains(results[0].Error, "--nth must be 1 or greater") {
		t.Fatalf("error = %q, want invalid nth error", results[0].Error)
	}
}

func TestBatchSnapshotRejectsArguments(t *testing.T) {
	err := validateBatchCommand("snapshot -i", batchOptions{page: 7, pageSet: true})
	if err == nil {
		t.Fatal("validateBatchCommand(snapshot -i) error = nil, want argument error")
	}
	if !strings.Contains(err.Error(), "snapshot does not take arguments") {
		t.Fatalf("error = %q, want snapshot argument error", err.Error())
	}
}

func TestBatchReadParsesCommandFlags(t *testing.T) {
	got, err := batchReadOptions([]string{"read", "--text", "--selector=.main", "--viewport", "--include-links", "--images"})
	if err != nil {
		t.Fatalf("batchReadOptions() error = %v", err)
	}
	want := readOptions{
		format:        "text",
		selector:      ".main",
		viewportOnly:  true,
		includeLinks:  true,
		includeImages: true,
	}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("read options = %#v, want %#v", got, want)
	}
}

func TestBatchGrepParsesCommandFlags(t *testing.T) {
	pattern, over, limit, err := batchGrepArgs([]string{"grep", "--content", "--limit=5", "price"})
	if err != nil {
		t.Fatalf("batchGrepArgs() error = %v", err)
	}
	if pattern != "price" || over != "content" || limit != 5 {
		t.Fatalf("grep args = %q %q %d, want price content 5", pattern, over, limit)
	}
}

func TestBatchPageRejectsNonPositivePage(t *testing.T) {
	if _, _, err := batchPage([]string{"-p", "0", "snapshot"}, batchOptions{}); err == nil {
		t.Fatal("batchPage() error = nil, want invalid page error")
	}
}

func TestSplitBatchCommandPreservesQuotedArgs(t *testing.T) {
	got, err := splitBatchCommand(`find text "Add to Cart" click`)
	if err != nil {
		t.Fatalf("splitBatchCommand() error = %v", err)
	}
	want := []string{"find", "text", "Add to Cart", "click"}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("tokens = %#v, want %#v", got, want)
	}
}

func TestSplitBatchCommandPreservesBackslashes(t *testing.T) {
	got, err := splitBatchCommand(`grep "\\d+"`)
	if err != nil {
		t.Fatalf("splitBatchCommand() error = %v", err)
	}
	want := []string{"grep", `\d+`}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("tokens = %#v, want %#v", got, want)
	}
}

func TestSplitBatchCommandPreservesEmptyQuotedArgs(t *testing.T) {
	got, err := splitBatchCommand(`fill @e1 ""`)
	if err != nil {
		t.Fatalf("splitBatchCommand() error = %v", err)
	}
	want := []string{"fill", "@e1", ""}
	if !reflect.DeepEqual(got, want) {
		t.Fatalf("tokens = %#v, want %#v", got, want)
	}
}
