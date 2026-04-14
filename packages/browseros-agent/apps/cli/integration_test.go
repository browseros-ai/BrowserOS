//go:build integration

package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"
)

var (
	cliBinary string
	serverURL string
)

func TestMain(m *testing.M) {
	serverURL = os.Getenv("BROWSEROS_URL")
	if serverURL == "" {
		serverURL = "http://127.0.0.1:9105"
	}

	client := &http.Client{Timeout: 3 * time.Second}
	resp, err := client.Get(serverURL + "/health")
	if err != nil {
		fmt.Fprintf(os.Stderr, "Skipping integration tests: server not reachable at %s\n", serverURL)
		os.Exit(0)
	}
	resp.Body.Close()

	tmpDir, err := os.MkdirTemp("", "browseros-cli-test-*")
	if err != nil {
		fmt.Fprintf(os.Stderr, "Failed to create temp dir: %v\n", err)
		os.Exit(1)
	}

	cliBinary = filepath.Join(tmpDir, "browseros-cli")
	buildCmd := exec.Command("go", "build", "-o", cliBinary, ".")
	buildCmd.Stderr = os.Stderr
	if err := buildCmd.Run(); err != nil {
		fmt.Fprintf(os.Stderr, "Failed to build CLI: %v\n", err)
		os.RemoveAll(tmpDir)
		os.Exit(1)
	}

	code := m.Run()
	os.RemoveAll(tmpDir)
	os.Exit(code)
}

type runResult struct {
	Stdout   string
	Stderr   string
	ExitCode int
}

func run(t *testing.T, args ...string) runResult {
	t.Helper()
	fullArgs := append([]string{"--server", serverURL}, args...)
	cmd := exec.Command(cliBinary, fullArgs...)
	var stdout, stderr strings.Builder
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr
	err := cmd.Run()
	code := 0
	if err != nil {
		if exitErr, ok := err.(*exec.ExitError); ok {
			code = exitErr.ExitCode()
		} else {
			t.Fatalf("exec error: %v", err)
		}
	}
	return runResult{
		Stdout:   stdout.String(),
		Stderr:   stderr.String(),
		ExitCode: code,
	}
}

func runJSON(t *testing.T, args ...string) map[string]any {
	t.Helper()
	fullArgs := append([]string{"--json"}, args...)
	r := run(t, fullArgs...)
	if r.ExitCode != 0 {
		t.Fatalf("command %v exited %d: %s%s", args, r.ExitCode, r.Stdout, r.Stderr)
	}
	var data map[string]any
	if err := json.Unmarshal([]byte(strings.TrimSpace(r.Stdout)), &data); err != nil {
		t.Fatalf("invalid JSON output for %v: %v\nraw: %s", args, err, r.Stdout)
	}
	return data
}

func TestHealth(t *testing.T) {
	data := runJSON(t, "health")
	status, ok := data["status"].(string)
	if !ok || status != "ok" {
		t.Errorf("expected status ok, got %v", data["status"])
	}
}

func TestVersion(t *testing.T) {
	r := run(t, "--version")
	if r.ExitCode != 0 {
		t.Fatalf("--version exited %d", r.ExitCode)
	}
	if !strings.Contains(r.Stdout, "browseros-cli") {
		t.Errorf("expected version output to contain 'browseros-cli', got: %s", r.Stdout)
	}
}

func TestPageLifecycle(t *testing.T) {
	// List existing pages
	pagesBefore := runJSON(t, "pages")
	countBefore, _ := pagesBefore["count"].(float64)
	if countBefore < 1 {
		t.Log("Warning: no pages found before test, server may not have a browser connected")
	}

	// Open a new page
	openData := runJSON(t, "open", "https://example.com")
	pageIDFloat, ok := openData["pageId"].(float64)
	if !ok {
		t.Fatalf("expected pageId in open response, got: %v", openData)
	}
	pageID := int(pageIDFloat)
	t.Logf("Opened page %d", pageID)

	pageArg := fmt.Sprintf("-p=%d", pageID)

	// Verify page count increased
	pagesAfter := runJSON(t, "pages")
	countAfter, _ := pagesAfter["count"].(float64)
	if countAfter <= countBefore {
		t.Errorf("expected page count to increase: before=%v after=%v", countBefore, countAfter)
	}

	// Wait for page to load
	time.Sleep(2 * time.Second)

	// Text extraction
	t.Run("text", func(t *testing.T) {
		data := runJSON(t, "text", pageArg)
		// structuredContent may have a "text" key or the content items have text
		// With --json, the output is structuredContent if present
		raw, _ := json.Marshal(data)
		if !strings.Contains(strings.ToLower(string(raw)), "example") {
			t.Errorf("expected page content to mention 'example', got: %s", string(raw))
		}
	})

	// Snapshot
	t.Run("snap", func(t *testing.T) {
		r := run(t, "--json", "snap", pageArg)
		if r.ExitCode != 0 {
			t.Fatalf("snap exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
		}
		if len(r.Stdout) < 10 {
			t.Errorf("snapshot output too short: %s", r.Stdout)
		}
	})

	// Eval
	t.Run("eval", func(t *testing.T) {
		r := run(t, "--json", "eval", pageArg, "document.title")
		if r.ExitCode != 0 {
			t.Fatalf("eval exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
		}
		out := strings.TrimSpace(r.Stdout)
		if !strings.Contains(strings.ToLower(out), "example") {
			t.Errorf("expected eval result to contain 'example', got: %s", out)
		}
	})

	// Screenshot
	t.Run("screenshot", func(t *testing.T) {
		r := run(t, "--json", "ss", pageArg)
		if r.ExitCode != 0 {
			t.Fatalf("ss exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
		}
		out := strings.TrimSpace(r.Stdout)
		// JSON output should contain image data or mimeType
		if !strings.Contains(out, "image") && !strings.Contains(out, "data") {
			t.Errorf("expected screenshot output to contain image data, got: %s", out[:min(len(out), 200)])
		}
	})

	// Navigate
	t.Run("nav", func(t *testing.T) {
		r := run(t, "--json", "nav", pageArg, "https://example.com/nav-test")
		if r.ExitCode != 0 {
			t.Fatalf("nav exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
		}
	})

	// Reload
	t.Run("reload", func(t *testing.T) {
		r := run(t, "--json", "reload", pageArg)
		if r.ExitCode != 0 {
			t.Fatalf("reload exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
		}
	})

	// Close the page (cleanup)
	closeR := run(t, "--json", "close", fmt.Sprintf("%d", pageID))
	if closeR.ExitCode != 0 {
		t.Errorf("close exited %d: %s%s", closeR.ExitCode, closeR.Stdout, closeR.Stderr)
	}
}

func TestActivePage(t *testing.T) {
	data := runJSON(t, "active")
	page, ok := data["page"].(map[string]any)
	if !ok {
		t.Fatalf("expected active page response to contain page object, got: %v", data)
	}
	if _, ok := page["pageId"].(float64); !ok {
		t.Fatalf("expected active page response to contain numeric pageId, got: %v", page)
	}
}

func TestSnapWithoutExplicitPage(t *testing.T) {
	activeData := runJSON(t, "active")
	page, ok := activeData["page"].(map[string]any)
	if !ok {
		t.Fatalf("expected active page response to contain page object, got: %v", activeData)
	}
	if _, ok := page["pageId"].(float64); !ok {
		t.Fatalf("expected active page response to contain numeric pageId, got: %v", page)
	}
	r := run(t, "--json", "snap")
	if r.ExitCode != 0 {
		t.Fatalf("snap exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
	}
	if len(strings.TrimSpace(r.Stdout)) < 10 {
		t.Fatalf("snapshot output too short: %s", r.Stdout)
	}
}

func TestInfo(t *testing.T) {
	r := run(t, "--json", "info")
	if r.ExitCode != 0 {
		t.Fatalf("info exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
	}
	if len(r.Stdout) < 5 {
		t.Errorf("info output too short: %s", r.Stdout)
	}
}

func TestEvalError(t *testing.T) {
	// Open a page for eval
	openData := runJSON(t, "open", "about:blank")
	pageID := int(openData["pageId"].(float64))
	defer run(t, "close", fmt.Sprintf("%d", pageID))

	r := run(t, "--json", "eval", fmt.Sprintf("-p=%d", pageID), "throw new Error('test-error')")
	if r.ExitCode == 0 {
		t.Errorf("expected eval with throw to exit non-zero")
	}
}

func TestInvalidPage(t *testing.T) {
	r := run(t, "--json", "snap", "-p=999999")
	if r.ExitCode == 0 {
		t.Errorf("expected snap with invalid page ID to exit non-zero")
	}
}

func TestStrataCheck(t *testing.T) {
	r := run(t, "--json", "strata", "check", "Gmail")
	// Klavis may not be configured — accept success or structured error
	out := strings.TrimSpace(r.Stdout + r.Stderr)
	if out == "" {
		t.Fatal("strata check produced no output")
	}
	if r.ExitCode == 0 {
		var data map[string]any
		if err := json.Unmarshal([]byte(strings.TrimSpace(r.Stdout)), &data); err != nil {
			t.Fatalf("strata check returned non-JSON: %s", r.Stdout)
		}
	}
}

func TestStrataDiscover(t *testing.T) {
	r := run(t, "--json", "strata", "discover", "send email", "Gmail")
	out := strings.TrimSpace(r.Stdout + r.Stderr)
	if out == "" {
		t.Fatal("strata discover produced no output")
	}
	if r.ExitCode == 0 {
		var data map[string]any
		if err := json.Unmarshal([]byte(strings.TrimSpace(r.Stdout)), &data); err != nil {
			t.Fatalf("strata discover returned non-JSON: %s", r.Stdout)
		}
	}
}

func TestStrataSearch(t *testing.T) {
	r := run(t, "--json", "strata", "search", "send email", "Gmail")
	out := strings.TrimSpace(r.Stdout + r.Stderr)
	if out == "" {
		t.Fatal("strata search produced no output")
	}
	if r.ExitCode == 0 {
		var data map[string]any
		if err := json.Unmarshal([]byte(strings.TrimSpace(r.Stdout)), &data); err != nil {
			t.Fatalf("strata search returned non-JSON: %s", r.Stdout)
		}
	}
}

func TestStrataActions(t *testing.T) {
	r := run(t, "--json", "strata", "actions", "Gmail")
	out := strings.TrimSpace(r.Stdout + r.Stderr)
	if out == "" {
		t.Fatal("strata actions produced no output")
	}
}

func TestStrataDetails(t *testing.T) {
	r := run(t, "--json", "strata", "details", "Gmail", "send_email")
	out := strings.TrimSpace(r.Stdout + r.Stderr)
	if out == "" {
		t.Fatal("strata details produced no output")
	}
}

func TestStrataAuth(t *testing.T) {
	r := run(t, "--json", "strata", "auth", "Gmail")
	out := strings.TrimSpace(r.Stdout + r.Stderr)
	if out == "" {
		t.Fatal("strata auth produced no output")
	}
}

func TestStrataExecMissingArgs(t *testing.T) {
	r := run(t, "strata", "exec")
	if r.ExitCode == 0 {
		t.Error("expected strata exec without args to exit non-zero")
	}
}

func TestStrataCheckMissingArgs(t *testing.T) {
	r := run(t, "strata", "check")
	if r.ExitCode == 0 {
		t.Error("expected strata check without args to exit non-zero")
	}
}
