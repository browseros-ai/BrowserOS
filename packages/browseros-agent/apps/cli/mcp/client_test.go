package mcp

import (
	"context"
	"fmt"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestConnectOptsIntoStructuredContent(t *testing.T) {
	var requestURI string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		requestURI = r.URL.RequestURI()
		http.Error(w, "stop after capturing endpoint", http.StatusInternalServerError)
	}))
	t.Cleanup(server.Close)

	client := NewClient(server.URL, "test", time.Second)
	_, _ = client.connect(context.Background())

	if requestURI != "/mcp?structured=1" {
		t.Fatalf("MCP request URI = %q, want /mcp?structured=1", requestURI)
	}
}

func TestHealthUsesCanonicalSystemHealthEndpoint(t *testing.T) {
	var paths []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		paths = append(paths, r.URL.Path)
		if r.URL.Path != "/system/health" {
			http.NotFound(w, r)
			return
		}
		fmt.Fprint(w, `{"status":"ok","cdpConnected":true}`)
	}))
	t.Cleanup(server.Close)

	data, err := NewClient(server.URL, "test", time.Second).Health()
	if err != nil {
		t.Fatalf("Health() error = %v", err)
	}
	if data["status"] != "ok" {
		t.Fatalf("Health() status = %v, want ok", data["status"])
	}
	assertPaths(t, paths, []string{"/system/health"})
}

func TestHealthFallsBackToLegacyRootHealthEndpoint(t *testing.T) {
	var paths []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		paths = append(paths, r.URL.Path)
		switch r.URL.Path {
		case "/system/health":
			http.NotFound(w, r)
		case "/health":
			fmt.Fprint(w, `{"status":"ok"}`)
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(server.Close)

	data, err := NewClient(server.URL, "test", time.Second).Health()
	if err != nil {
		t.Fatalf("Health() error = %v", err)
	}
	if data["status"] != "ok" {
		t.Fatalf("Health() status = %v, want ok", data["status"])
	}
	assertPaths(t, paths, []string{"/system/health", "/health"})
}

func TestHealthDoesNotFallbackOnCanonicalHealthFailure(t *testing.T) {
	var paths []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		paths = append(paths, r.URL.Path)
		if r.URL.Path == "/system/health" {
			http.Error(w, "boom", http.StatusInternalServerError)
			return
		}
		http.NotFound(w, r)
	}))
	t.Cleanup(server.Close)

	_, err := NewClient(server.URL, "test", time.Second).Health()
	if err == nil {
		t.Fatal("Health() error = nil, want HTTP 500")
	}
	if !strings.Contains(err.Error(), "HTTP 500") {
		t.Fatalf("Health() error = %q, want HTTP 500", err)
	}
	assertPaths(t, paths, []string{"/system/health"})
}

func TestStatusUsesRuntimeStatusEndpoint(t *testing.T) {
	var paths []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		paths = append(paths, r.URL.Path)
		if r.URL.Path != "/status" {
			http.NotFound(w, r)
			return
		}
		fmt.Fprint(w, `{"status":"ok","cdpConnected":true,"can_update":true}`)
	}))
	t.Cleanup(server.Close)

	data, err := NewClient(server.URL, "test", time.Second).Status()
	if err != nil {
		t.Fatalf("Status() error = %v", err)
	}
	if data["can_update"] != true {
		t.Fatalf("Status() can_update = %v, want true", data["can_update"])
	}
	assertPaths(t, paths, []string{"/status"})
}

func TestStatusFallsBackToCompatibleHealthEndpoint(t *testing.T) {
	var paths []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		paths = append(paths, r.URL.Path)
		switch r.URL.Path {
		case "/status":
			http.NotFound(w, r)
		case "/system/health":
			fmt.Fprint(w, `{"status":"ok","cdpConnected":true}`)
		default:
			http.NotFound(w, r)
		}
	}))
	t.Cleanup(server.Close)

	data, err := NewClient(server.URL, "test", time.Second).Status()
	if err != nil {
		t.Fatalf("Status() error = %v", err)
	}
	if data["cdpConnected"] != true {
		t.Fatalf("Status() cdpConnected = %v, want true", data["cdpConnected"])
	}
	assertPaths(t, paths, []string{"/status", "/system/health"})
}

func TestStatusDoesNotFallbackOnRuntimeStatusFailure(t *testing.T) {
	var paths []string
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		paths = append(paths, r.URL.Path)
		if r.URL.Path == "/status" {
			http.Error(w, "boom", http.StatusInternalServerError)
			return
		}
		http.NotFound(w, r)
	}))
	t.Cleanup(server.Close)

	_, err := NewClient(server.URL, "test", time.Second).Status()
	if err == nil {
		t.Fatal("Status() error = nil, want HTTP 500")
	}
	if !strings.Contains(err.Error(), "HTTP 500") {
		t.Fatalf("Status() error = %q, want HTTP 500", err)
	}
	assertPaths(t, paths, []string{"/status"})
}

func assertPaths(t *testing.T, got, want []string) {
	t.Helper()

	if len(got) != len(want) {
		t.Fatalf("paths = %v, want %v", got, want)
	}
	for i := range got {
		if got[i] != want[i] {
			t.Fatalf("paths = %v, want %v", got, want)
		}
	}
}
