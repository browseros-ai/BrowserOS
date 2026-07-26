package raw

import (
	"encoding/json"
	"errors"
	"fmt"
	"os/exec"
	"reflect"
	"strings"
	"testing"

	"browseros-cli/mcp"
)

type fakeClient struct {
	calls  []toolCall
	result *mcp.ToolResult
	err    error
}

type toolCall struct {
	name string
	args map[string]any
}

func (f *fakeClient) CallTool(name string, args map[string]any) (*mcp.ToolResult, error) {
	f.calls = append(f.calls, toolCall{name: name, args: args})
	return f.result, f.err
}

func TestExecuteCDPRejectsInvalidJSONBeforeRun(t *testing.T) {
	client := &fakeClient{}
	deps := Deps{
		NewClient:     func() Client { return client },
		ResolvePageID: func() (int, error) { return 7, nil },
	}

	_, err := executeCDP(deps, "Runtime.evaluate", "{")
	if err == nil {
		t.Fatal("executeCDP() error = nil, want JSON error")
	}
	if !strings.Contains(err.Error(), "invalid JSON params") {
		t.Fatalf("error = %q, want invalid JSON params", err)
	}
	if len(client.calls) != 0 {
		t.Fatalf("run calls = %d, want 0", len(client.calls))
	}
}

func TestExecuteCDPRequiresPageBeforeRun(t *testing.T) {
	client := &fakeClient{}
	deps := Deps{
		NewClient:     func() Client { return client },
		ResolvePageID: func() (int, error) { return 0, errors.New("page id is required: pass -p/--page <id>") },
	}

	_, err := executeCDP(deps, "Runtime.evaluate", "{}")
	if err == nil {
		t.Fatal("executeCDP() error = nil, want page error")
	}
	if !strings.Contains(err.Error(), "-p/--page") {
		t.Fatalf("error = %q, want explicit page guidance", err)
	}
	if len(client.calls) != 0 {
		t.Fatalf("run calls = %d, want 0", len(client.calls))
	}
}

func TestCallCDPBuildsPageSessionScript(t *testing.T) {
	client := &fakeClient{result: runResult(true, map[string]any{"result": float64(3)}, []string{"log one"}, "")}
	params := mustParseParams(t, `{"expression":"document.title","returnByValue":true}`)

	result, err := callCDP(client, 12, "Runtime.evaluate", params)
	if err != nil {
		t.Fatalf("callCDP() error = %v", err)
	}
	if len(client.calls) != 1 {
		t.Fatalf("run calls = %d, want 1", len(client.calls))
	}

	call := client.calls[0]
	if call.name != "run" {
		t.Fatalf("tool = %q, want run", call.name)
	}
	code, ok := call.args["code"].(string)
	if !ok {
		t.Fatalf("run code = %#v, want string", call.args["code"])
	}
	for _, want := range []string{
		"const page = 12",
		`const method = "Runtime.evaluate"`,
		`const paramsJson = "{\"expression\":\"document.title\",\"returnByValue\":true}"`,
		"browser.cdpJsonForPage(page, method, paramsJson)",
	} {
		if !strings.Contains(code, want) {
			t.Fatalf("generated code missing %q in:\n%s", want, code)
		}
	}

	wantStructured := map[string]any{
		"ok":     true,
		"page":   12,
		"method": "Runtime.evaluate",
		"result": map[string]any{"result": float64(3)},
		"logs":   []string{"log one"},
	}
	if !reflect.DeepEqual(result.StructuredContent, wantStructured) {
		t.Fatalf("structured = %#v, want %#v", result.StructuredContent, wantStructured)
	}
}

func TestRunJSPreservesErrorEnvelope(t *testing.T) {
	client := &fakeClient{
		result: runResult(false, nil, []string{"before failure"}, `Unknown CDP method "Nope.nope"`),
		err:    errors.New(`error: Unknown CDP method "Nope.nope"`),
	}

	result, err := callCDP(client, 3, "Nope.nope", json.RawMessage(`{}`))
	if err == nil {
		t.Fatal("callCDP() error = nil, want run error")
	}
	if !strings.Contains(err.Error(), `Unknown CDP method "Nope.nope"`) {
		t.Fatalf("error = %q, want real CDP method error", err)
	}
	if !strings.Contains(err.Error(), "before failure") {
		t.Fatalf("error = %q, want captured logs", err)
	}
	if strings.Contains(err.Error(), "structured value") || strings.Contains(err.Error(), "missing") {
		t.Fatalf("error = %q, want protocol/server error, not missing-value error", err)
	}
	if result == nil {
		t.Fatal("result = nil, want structured error result")
	}
	wantStructured := map[string]any{
		"ok":     false,
		"page":   3,
		"method": "Nope.nope",
		"result": nil,
		"logs":   []string{"before failure"},
		"error":  `Unknown CDP method "Nope.nope"`,
	}
	if !reflect.DeepEqual(result.StructuredContent, wantStructured) {
		t.Fatalf("structured = %#v, want %#v", result.StructuredContent, wantStructured)
	}
	if !result.IsError {
		t.Fatal("result.IsError = false, want true")
	}
}

func TestParseRunEnvelopeAcceptsValueAndLogs(t *testing.T) {
	env, err := parseRunEnvelope(runResult(true, "ok-value", []string{}, ""))
	if err != nil {
		t.Fatalf("parseRunEnvelope() error = %v", err)
	}
	if !env.OK || env.Value != "ok-value" || len(env.Logs) != 0 || env.Error != "" {
		t.Fatalf("envelope = %#v, want ok value with empty logs", env)
	}
}

func TestParseParamsForwardsAnyValidJSON(t *testing.T) {
	got, err := parseParams(`{"id":9007199254740993123456789,"items":[{"x":1},null,true]}`)
	if err != nil {
		t.Fatalf("parseParams() error = %v", err)
	}
	if string(got) != `{"id":9007199254740993123456789,"items":[{"x":1},null,true]}` {
		t.Fatalf("params = %s, want raw JSON preserved", string(got))
	}

	code := cdpScript(4, "Runtime.evaluate", got)
	if !strings.Contains(code, `9007199254740993123456789`) {
		t.Fatalf("generated code rewrote large integer:\n%s", code)
	}
	if strings.Contains(code, "JSON.parse") {
		t.Fatalf("generated code parses params before forwarding:\n%s", code)
	}
}

func TestCDPScriptRejectsInheritedDomain(t *testing.T) {
	result, err := runCDPScriptWithNode(t, cdpScript(4, "constructor.keys", json.RawMessage(`{}`)))
	if err == nil {
		t.Fatal("script succeeded, want inherited domain rejection")
	}
	if result.OK {
		t.Fatalf("script result OK = true, want false: %+v", result)
	}
	if !strings.Contains(result.Message, `Unknown CDP method "constructor.keys"`) {
		t.Fatalf("message = %q, want unknown method", result.Message)
	}
}

func TestCDPScriptAllowsOwnDomainMethod(t *testing.T) {
	result, err := runCDPScriptWithNode(t, cdpScript(4, "Runtime.evaluate", json.RawMessage(`{"returnByValue":true}`)))
	if err != nil {
		t.Fatalf("script error = %v; result = %+v", err, result)
	}
	if !result.OK {
		t.Fatalf("script result OK = false: %+v", result)
	}
	if got := result.Value["ok"]; got != true {
		t.Fatalf("script value ok = %#v, want true", got)
	}
}

func mustParseParams(t *testing.T, raw string) json.RawMessage {
	t.Helper()
	params, err := parseParams(raw)
	if err != nil {
		t.Fatalf("parseParams() error = %v", err)
	}
	return params
}

type nodeScriptResult struct {
	OK      bool           `json:"ok"`
	Message string         `json:"message"`
	Value   map[string]any `json:"value"`
}

func runCDPScriptWithNode(t *testing.T, code string) (nodeScriptResult, error) {
	t.Helper()

	node, err := exec.LookPath("node")
	if err != nil {
		t.Skip("node is not available")
	}

	codeLiteral, _ := json.Marshal(code)
	script := fmt.Sprintf(`const AsyncFunction = Object.getPrototypeOf(async () => {}).constructor;
const code = %s;
const browser = {
  cdpJsonForPage: async (page, method, paramsJson) => {
    if (method === 'constructor.keys') {
      throw new Error('Unknown CDP method "constructor.keys"');
    }
    return { ok: true, page, method, paramsJson };
  }
};
new AsyncFunction('browser', code)(browser)
  .then((value) => console.log(JSON.stringify({ ok: true, value })))
  .catch((err) => {
    console.log(JSON.stringify({ ok: false, message: err.message }));
    process.exit(1);
  });`, string(codeLiteral))

	output, runErr := exec.Command(node, "-e", script).CombinedOutput()
	var result nodeScriptResult
	if err := json.Unmarshal(output, &result); err != nil {
		t.Fatalf("json.Unmarshal(%q) error = %v", string(output), err)
	}
	return result, runErr
}

func runResult(ok bool, value any, logs []string, errText string) *mcp.ToolResult {
	structured := map[string]any{
		"ok":   ok,
		"logs": logs,
	}
	if value != nil {
		structured["value"] = value
	}
	if errText != "" {
		structured["error"] = errText
	}
	text := "ok"
	if !ok {
		text = "error: " + errText
	}
	return &mcp.ToolResult{
		Content:           []mcp.ContentItem{{Type: "text", Text: text}},
		StructuredContent: structured,
		IsError:           !ok,
	}
}
