//go:build integration

package main

import (
	"encoding/json"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
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

func fixtureURL(html string) string {
	return "data:text/html;charset=utf-8," + url.PathEscape(html)
}

func openedPage(t *testing.T, data map[string]any) int {
	t.Helper()
	page, ok := data["page"].(float64)
	if !ok {
		t.Fatalf("expected page in open response, got: %v", data)
	}
	return int(page)
}

func markdownLinkTargets(markdown string) []string {
	matches := regexp.MustCompile(`\]\(([^)]+)\)`).FindAllStringSubmatch(markdown, -1)
	targets := make([]string, 0, len(matches))
	for _, match := range matches {
		targets = append(targets, match[1])
	}
	return targets
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
	pagesBefore := runJSON(t, "tabs")
	countBefore, _ := pagesBefore["count"].(float64)
	if countBefore < 1 {
		t.Log("Warning: no pages found before test, server may not have a browser connected")
	}

	openData := runJSON(t, "open", fixtureURL(`<title>Example Fixture</title><main><h1>Example</h1><p>Hello from BrowserOS.</p><button>Submit</button></main>`))
	pageID := openedPage(t, openData)
	t.Logf("Opened page %d", pageID)

	pageArg := fmt.Sprintf("-p=%d", pageID)

	pagesAfter := runJSON(t, "tabs")
	countAfter, _ := pagesAfter["count"].(float64)
	if countAfter <= countBefore {
		t.Errorf("expected page count to increase: before=%v after=%v", countBefore, countAfter)
	}

	time.Sleep(2 * time.Second)

	t.Run("text", func(t *testing.T) {
		data := runJSON(t, "text", pageArg)
		raw, _ := json.Marshal(data)
		if !strings.Contains(strings.ToLower(string(raw)), "example") {
			t.Errorf("expected page content to mention 'example', got: %s", string(raw))
		}
	})

	t.Run("snap", func(t *testing.T) {
		r := run(t, "--json", "snap", pageArg)
		if r.ExitCode != 0 {
			t.Fatalf("snap exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
		}
		if len(r.Stdout) < 10 {
			t.Errorf("snapshot output too short: %s", r.Stdout)
		}
	})

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

	t.Run("screenshot", func(t *testing.T) {
		r := run(t, "--json", "screenshot", pageArg)
		if r.ExitCode != 0 {
			t.Fatalf("ss exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
		}
		out := strings.TrimSpace(r.Stdout)
		// JSON output should contain image data or mimeType
		if !strings.Contains(out, "image") && !strings.Contains(out, "data") {
			t.Errorf("expected screenshot output to contain image data, got: %s", out[:min(len(out), 200)])
		}
	})

	t.Run("nav", func(t *testing.T) {
		r := run(t, "--json", "nav", pageArg, fixtureURL(`<title>Nav Fixture</title><p>navigated</p>`))
		if r.ExitCode != 0 {
			t.Fatalf("nav exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
		}
	})

	t.Run("reload", func(t *testing.T) {
		r := run(t, "--json", "reload", pageArg)
		if r.ExitCode != 0 {
			t.Fatalf("reload exited %d: %s%s", r.ExitCode, r.Stdout, r.Stderr)
		}
	})

	closeR := run(t, "--json", "close", pageArg)
	if closeR.ExitCode != 0 {
		t.Errorf("close exited %d: %s%s", closeR.ExitCode, closeR.Stdout, closeR.Stderr)
	}
}

func TestActivePage(t *testing.T) {
	data := runJSON(t, "active")
	if _, ok := data["page"].(float64); !ok {
		t.Fatalf("expected active page response to contain numeric page, got: %v", data)
	}
	if _, ok := data["tabId"].(float64); !ok {
		t.Fatalf("expected active page response to contain numeric tabId, got: %v", data)
	}
}

func TestSnapWithoutExplicitPageFails(t *testing.T) {
	r := run(t, "--json", "snap")
	if r.ExitCode == 0 {
		t.Fatalf("snap without -p succeeded: %s", r.Stdout)
	}
	if !strings.Contains(r.Stderr, "-p/--page") {
		t.Fatalf("missing-page error = %q, want -p/--page guidance", r.Stderr)
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
	openData := runJSON(t, "open", "about:blank")
	pageID := openedPage(t, openData)
	defer run(t, "close", fmt.Sprintf("-p=%d", pageID))

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

func TestExplicitPageAgentFlows(t *testing.T) {
	openData := runJSON(t, "open", fixtureURL(`<title>Search Fixture</title><label>Search <input aria-label="Search" autofocus></label><button type="button">Go</button>`))
	pageID := openedPage(t, openData)
	pageArg := fmt.Sprintf("-p=%d", pageID)
	defer run(t, "close", pageArg)

	batchR := run(t, "--json", "batch", pageArg, "--bail", "find role textbox --name Search fill batch-query", "press Enter", "snapshot")
	if batchR.ExitCode != 0 {
		t.Fatalf("batch exited %d: %s%s", batchR.ExitCode, batchR.Stdout, batchR.Stderr)
	}
	var batchResults []struct {
		OK bool `json:"ok"`
	}
	if err := json.Unmarshal([]byte(batchR.Stdout), &batchResults); err != nil {
		t.Fatalf("batch JSON parse failed: %v\n%s", err, batchR.Stdout)
	}
	if len(batchResults) != 3 {
		t.Fatalf("batch results = %d, want 3", len(batchResults))
	}
	for i, result := range batchResults {
		if !result.OK {
			t.Fatalf("batch result %d failed: %s", i+1, batchR.Stdout)
		}
	}

	findR := run(t, "--json", "find", pageArg, "role", "textbox", "--name", "Search", "fill", "sensodyne")
	if findR.ExitCode != 0 {
		t.Fatalf("find fill exited %d: %s%s", findR.ExitCode, findR.Stdout, findR.Stderr)
	}
	pressR := run(t, "--json", "press", pageArg, "Enter")
	if pressR.ExitCode != 0 {
		t.Fatalf("press exited %d: %s%s", pressR.ExitCode, pressR.Stdout, pressR.Stderr)
	}
	snapshotR := run(t, "--json", "snapshot", pageArg)
	if snapshotR.ExitCode != 0 {
		t.Fatalf("snapshot exited %d: %s%s", snapshotR.ExitCode, snapshotR.Stdout, snapshotR.Stderr)
	}
}

func TestExplicitPageFanOutFlow(t *testing.T) {
	first := fixtureURL(`<title>First Link</title><p>first page</p>`)
	second := fixtureURL(`<title>Second Link</title><p>second page</p>`)
	indexHTML := fmt.Sprintf(`<title>Links Fixture</title><a href=%q>First</a><a href=%q>Second</a>`, first, second)
	openData := runJSON(t, "open", fixtureURL(indexHTML))
	pageID := openedPage(t, openData)
	pageArg := fmt.Sprintf("-p=%d", pageID)
	defer run(t, "close", pageArg)

	linksR := run(t, "read", pageArg, "--links")
	if linksR.ExitCode != 0 {
		t.Fatalf("read --links exited %d: %s%s", linksR.ExitCode, linksR.Stdout, linksR.Stderr)
	}
	targets := markdownLinkTargets(linksR.Stdout)
	if len(targets) < 2 {
		t.Fatalf("expected at least two link targets, got %v from %q", targets, linksR.Stdout)
	}

	for _, target := range targets[:2] {
		childData := runJSON(t, "open", target)
		childPage := openedPage(t, childData)
		childPageArg := fmt.Sprintf("-p=%d", childPage)
		readR := run(t, "read", childPageArg, "--text")
		if readR.ExitCode != 0 {
			t.Fatalf("read child exited %d: %s%s", readR.ExitCode, readR.Stdout, readR.Stderr)
		}
		run(t, "close", childPageArg)
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
