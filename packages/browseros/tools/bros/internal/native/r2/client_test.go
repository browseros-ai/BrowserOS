package r2

import (
	"reflect"
	"testing"
)

func TestLoadConfigDefaults(t *testing.T) {
	cfg := LoadConfig(func(string) string { return "" })

	if cfg.Bucket != DefaultR2Bucket {
		t.Fatalf("expected bucket %q, got %q", DefaultR2Bucket, cfg.Bucket)
	}
	if cfg.CDNBaseURL != DefaultR2CDNBaseURL {
		t.Fatalf("expected cdn base %q, got %q", DefaultR2CDNBaseURL, cfg.CDNBaseURL)
	}
	if cfg.EndpointURL != "" {
		t.Fatalf("expected empty endpoint URL, got %q", cfg.EndpointURL)
	}
}

func TestLoadConfigEndpointDerivedFromAccountID(t *testing.T) {
	cfg := LoadConfig(func(key string) string {
		switch key {
		case R2AccountIDEnv:
			return "abc123"
		case R2AccessKeyIDEnv:
			return "access"
		case R2SecretAccessKeyEnv:
			return "secret"
		default:
			return ""
		}
	})

	if cfg.EndpointURL != "https://abc123.r2.cloudflarestorage.com" {
		t.Fatalf("unexpected endpoint: %q", cfg.EndpointURL)
	}
	if !cfg.HasConfig() {
		t.Fatal("expected HasConfig true")
	}
	if err := cfg.Validate(); err != nil {
		t.Fatalf("Validate returned error: %v", err)
	}
}

func TestSortVersionsDesc(t *testing.T) {
	versions := []string{"0.9.0", "0.10.0", "1.0.0", "0.31.0", "0.31.0.1"}

	SortVersionsDesc(versions)

	expected := []string{"1.0.0", "0.31.0.1", "0.31.0", "0.10.0", "0.9.0"}
	if !reflect.DeepEqual(versions, expected) {
		t.Fatalf("unexpected order: got %v want %v", versions, expected)
	}
}

func TestCompareVersions(t *testing.T) {
	cases := []struct {
		a, b string
		cmp  int
	}{
		{a: "0.31.0", b: "0.30.9", cmp: 1},
		{a: "0.31.0", b: "0.31.0", cmp: 0},
		{a: "0.31.0", b: "0.31.1", cmp: -1},
		{a: "1", b: "1.0.0", cmp: 0},
	}

	for _, tc := range cases {
		if got := CompareVersions(tc.a, tc.b); got != tc.cmp {
			t.Fatalf("CompareVersions(%q, %q)=%d want %d", tc.a, tc.b, got, tc.cmp)
		}
	}
}

func TestCopySourceEscapesKey(t *testing.T) {
	got := copySource("bucket", "releases/0.31.0/macos/Browser OS.dmg")
	want := "bucket/releases/0.31.0/macos/Browser%20OS.dmg"
	if got != want {
		t.Fatalf("copySource mismatch: got %q want %q", got, want)
	}
}
