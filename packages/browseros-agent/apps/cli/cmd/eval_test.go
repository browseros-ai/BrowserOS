package cmd

import (
	"encoding/json"
	"fmt"
	"os/exec"
	"testing"
)

func TestEvalCodeSupportsExpressionsAndSnippets(t *testing.T) {
	tests := []struct {
		name   string
		source string
		want   any
	}{
		{
			name:   "expression",
			source: "1 + 2",
			want:   float64(3),
		},
		{
			name:   "statement completion",
			source: "let x = 4; x + 1",
			want:   float64(5),
		},
		{
			name:   "async body return",
			source: "return await Promise.resolve('ok')",
			want:   "ok",
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := runEvalCode(t, tt.source)
			if got != tt.want {
				t.Fatalf("runEvalCode() = %#v, want %#v", got, tt.want)
			}
		})
	}
}

func TestEvalCodeDoesNotHideRuntimeErrors(t *testing.T) {
	result, err := runEvalCodeResult(t, "missingVariable")
	if err == nil {
		t.Fatal("runEvalCodeResult() error = nil, want runtime failure")
	}
	if result.Name != "ReferenceError" {
		t.Fatalf("error name = %q, want ReferenceError", result.Name)
	}
}

func runEvalCode(t *testing.T, source string) any {
	t.Helper()

	result, err := runEvalCodeResult(t, source)
	if err != nil {
		t.Fatalf("runEvalCodeResult() error = %v; result = %+v", err, result)
	}
	return result.Value
}

type evalCodeResult struct {
	OK      bool   `json:"ok"`
	Value   any    `json:"value"`
	Name    string `json:"name"`
	Message string `json:"message"`
}

func runEvalCodeResult(t *testing.T, source string) (evalCodeResult, error) {
	t.Helper()

	node, err := exec.LookPath("node")
	if err != nil {
		t.Skip("node is not available")
	}

	script := fmt.Sprintf(`(async () => {
%s
})()
  .then((value) => console.log(JSON.stringify({ ok: true, value })))
  .catch((err) => {
    console.log(JSON.stringify({ ok: false, name: err.name, message: err.message }))
    process.exit(1)
  })`, evalCode(source))
	output, runErr := exec.Command(node, "-e", script).CombinedOutput()

	var result evalCodeResult
	if err := json.Unmarshal(output, &result); err != nil {
		t.Fatalf("json.Unmarshal(%q) error = %v", string(output), err)
	}
	return result, runErr
}
