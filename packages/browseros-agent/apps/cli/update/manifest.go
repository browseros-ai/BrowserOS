package update

import (
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"strings"

	"golang.org/x/mod/semver"
)

const maxManifestSize = 1 << 20

type Manifest struct {
	Version     string           `json:"version"`
	PublishedAt string           `json:"published_at"`
	Tag         string           `json:"tag"`
	Assets      map[string]Asset `json:"assets"`
}

type Asset struct {
	Filename      string `json:"filename"`
	URL           string `json:"url"`
	ArchiveFormat string `json:"archive_format"`
	SHA256        string `json:"sha256"`
}

func FetchManifest(
	ctx context.Context,
	client *http.Client,
	url string,
) (*Manifest, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}

	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("update manifest returned HTTP %d", resp.StatusCode)
	}

	var manifest Manifest
	if err := json.NewDecoder(io.LimitReader(resp.Body, maxManifestSize)).Decode(&manifest); err != nil {
		return nil, err
	}
	if err := manifest.Validate(); err != nil {
		return nil, err
	}

	return &manifest, nil
}

func (m *Manifest) Validate() error {
	if m == nil {
		return fmt.Errorf("update manifest is nil")
	}
	if !IsReleaseVersion(m.Version) {
		return fmt.Errorf("invalid manifest version %q", m.Version)
	}
	if len(m.Assets) == 0 {
		return fmt.Errorf("update manifest has no assets")
	}

	for key, asset := range m.Assets {
		if asset.URL == "" {
			return fmt.Errorf("asset %q is missing url", key)
		}
		if asset.SHA256 == "" {
			return fmt.Errorf("asset %q is missing sha256", key)
		}
		if asset.ArchiveFormat != "tar.gz" && asset.ArchiveFormat != "zip" {
			return fmt.Errorf("asset %q has unsupported archive format %q", key, asset.ArchiveFormat)
		}
	}

	return nil
}

func NormalizeVersion(version string) string {
	value := strings.TrimSpace(version)
	if value == "" {
		return ""
	}
	if !strings.HasPrefix(value, "v") {
		value = "v" + value
	}
	return semver.Canonical(value)
}

func IsReleaseVersion(version string) bool {
	return NormalizeVersion(version) != ""
}

func CompareVersions(current, latest string) (int, error) {
	normalizedCurrent := NormalizeVersion(current)
	if normalizedCurrent == "" {
		return 0, fmt.Errorf("invalid current version %q", current)
	}

	normalizedLatest := NormalizeVersion(latest)
	if normalizedLatest == "" {
		return 0, fmt.Errorf("invalid latest version %q", latest)
	}

	return semver.Compare(normalizedCurrent, normalizedLatest), nil
}

func PlatformKey(goos, goarch string) (string, error) {
	switch goos {
	case "darwin", "linux", "windows":
	default:
		return "", fmt.Errorf("unsupported os %q", goos)
	}

	switch goarch {
	case "amd64", "arm64":
	default:
		return "", fmt.Errorf("unsupported arch %q", goarch)
	}

	return goos + "/" + goarch, nil
}

func SelectAsset(manifest *Manifest, goos, goarch string) (Asset, error) {
	key, err := PlatformKey(goos, goarch)
	if err != nil {
		return Asset{}, err
	}

	asset, ok := manifest.Assets[key]
	if !ok {
		return Asset{}, fmt.Errorf("no update asset for %s", key)
	}

	return asset, nil
}
