package update

import (
	"context"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestNormalizeVersion(t *testing.T) {
	if got := NormalizeVersion("1.2.3"); got != "v1.2.3" {
		t.Fatalf("NormalizeVersion() = %q, want %q", got, "v1.2.3")
	}
	if got := NormalizeVersion("dev"); got != "" {
		t.Fatalf("NormalizeVersion(dev) = %q, want empty", got)
	}
}

func TestCompareVersions(t *testing.T) {
	got, err := CompareVersions("1.2.3", "1.3.0")
	if err != nil {
		t.Fatalf("CompareVersions() error = %v", err)
	}
	if got >= 0 {
		t.Fatalf("CompareVersions() = %d, want < 0", got)
	}
}

func TestSelectAsset(t *testing.T) {
	manifest := &Manifest{
		Version: "1.2.3",
		Assets: map[string]Asset{
			"darwin/arm64": {
				URL:           "https://cdn.example.com/cli/v1.2.3/browseros-cli.tar.gz",
				ArchiveFormat: "tar.gz",
				SHA256:        "abc",
			},
		},
	}

	asset, err := SelectAsset(manifest, "darwin", "arm64")
	if err != nil {
		t.Fatalf("SelectAsset() error = %v", err)
	}
	if asset.URL == "" {
		t.Fatal("SelectAsset() returned empty URL")
	}
}

func TestFetchManifest(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{
			"version":"1.2.3",
			"published_at":"2026-03-27T19:00:00Z",
			"tag":"browseros-cli-v1.2.3",
			"assets":{
				"darwin/arm64":{
					"filename":"browseros-cli_1.2.3_darwin_arm64.tar.gz",
					"url":"https://cdn.example.com/cli/v1.2.3/browseros-cli_1.2.3_darwin_arm64.tar.gz",
					"archive_format":"tar.gz",
					"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
				}
			}
		}`))
	}))
	defer server.Close()

	manifest, err := FetchManifest(context.Background(), server.Client(), server.URL)
	if err != nil {
		t.Fatalf("FetchManifest() error = %v", err)
	}
	if manifest.Version != "1.2.3" {
		t.Fatalf("FetchManifest() version = %q, want %q", manifest.Version, "1.2.3")
	}
}

func TestFetchManifestRejectsOversizedResponse(t *testing.T) {
	hugeName := strings.Repeat("a", maxManifestSize)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		_, _ = w.Write([]byte(`{
			"version":"1.2.3",
			"published_at":"2026-03-27T19:00:00Z",
			"tag":"browseros-cli-v1.2.3",
			"assets":{
				"darwin/arm64":{
					"filename":"` + hugeName + `",
					"url":"https://cdn.example.com/cli/v1.2.3/browseros-cli_1.2.3_darwin_arm64.tar.gz",
					"archive_format":"tar.gz",
					"sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
				}
			}
		}`))
	}))
	defer server.Close()

	if _, err := FetchManifest(context.Background(), server.Client(), server.URL); err == nil {
		t.Fatal("FetchManifest() error = nil, want oversized response error")
	}
}
