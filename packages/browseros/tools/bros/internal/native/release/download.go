package release

import (
	"context"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"time"
)

type DownloadOptions struct {
	OSFilter   string
	OutputDir  string
	HTTPClient *http.Client
}

type DownloadTarget struct {
	Platform    string
	ArtifactKey string
	Filename    string
	URL         string
	Path        string
	Expected    int64
}

type DownloadResult struct {
	DownloadTarget
	BytesWritten int64
	Err          error
}

type DownloadSummary struct {
	Directory string
	Results   []DownloadResult
}

func ResolveDownloadDirectory(version, outputDir string) string {
	if outputDir != "" {
		return filepath.Join(outputDir, version)
	}
	return filepath.Join(os.TempDir(), "browseros-releases", version)
}

func ResolveDownloadPlatforms(osFilter string) ([]string, error) {
	normalized, err := NormalizeOSFilter(osFilter)
	if err != nil {
		return nil, err
	}
	if normalized == "" {
		return Platforms, nil
	}
	return []string{normalized}, nil
}

func BuildDownloadTargets(metadata map[string]PlatformMetadata, platforms []string, downloadDir string) []DownloadTarget {
	targets := make([]DownloadTarget, 0)

	for _, platform := range platforms {
		releaseData, ok := metadata[platform]
		if !ok {
			continue
		}

		artifactKeys := make([]string, 0, len(releaseData.Artifacts))
		for key := range releaseData.Artifacts {
			artifactKeys = append(artifactKeys, key)
		}
		sort.Strings(artifactKeys)

		for _, artifactKey := range artifactKeys {
			artifact := releaseData.Artifacts[artifactKey]
			if artifact.Filename == "" || artifact.URL == "" {
				continue
			}

			targets = append(targets, DownloadTarget{
				Platform:    platform,
				ArtifactKey: artifactKey,
				Filename:    artifact.Filename,
				URL:         artifact.URL,
				Path:        filepath.Join(downloadDir, artifact.Filename),
				Expected:    artifact.Size,
			})
		}
	}

	return targets
}

func DownloadArtifacts(ctx context.Context, version string, metadata map[string]PlatformMetadata, opts DownloadOptions) (DownloadSummary, error) {
	if len(metadata) == 0 {
		return DownloadSummary{}, fmt.Errorf("no release metadata found for version %s", version)
	}

	platforms, err := ResolveDownloadPlatforms(opts.OSFilter)
	if err != nil {
		return DownloadSummary{}, err
	}

	downloadDir := ResolveDownloadDirectory(version, opts.OutputDir)
	if err := os.MkdirAll(downloadDir, 0o755); err != nil {
		return DownloadSummary{}, err
	}

	targets := BuildDownloadTargets(metadata, platforms, downloadDir)
	client := opts.HTTPClient
	if client == nil {
		client = &http.Client{Timeout: 30 * time.Second}
	}

	results := make([]DownloadResult, 0, len(targets))
	failed := 0
	for _, target := range targets {
		written, downloadErr := downloadFile(ctx, client, target.URL, target.Path)
		if downloadErr != nil {
			failed++
		}
		results = append(results, DownloadResult{
			DownloadTarget: target,
			BytesWritten:   written,
			Err:            downloadErr,
		})
	}

	summary := DownloadSummary{
		Directory: downloadDir,
		Results:   results,
	}
	if failed > 0 {
		return summary, fmt.Errorf("%d artifact download(s) failed", failed)
	}
	return summary, nil
}

func downloadFile(ctx context.Context, client *http.Client, sourceURL, destinationPath string) (int64, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, sourceURL, nil)
	if err != nil {
		return 0, err
	}

	resp, err := client.Do(req)
	if err != nil {
		return 0, err
	}
	defer resp.Body.Close()

	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return 0, fmt.Errorf("download %s: unexpected status %d", sourceURL, resp.StatusCode)
	}

	if err := os.MkdirAll(filepath.Dir(destinationPath), 0o755); err != nil {
		return 0, err
	}

	file, err := os.Create(destinationPath)
	if err != nil {
		return 0, err
	}
	defer file.Close()

	written, err := io.Copy(file, resp.Body)
	if err != nil {
		return written, err
	}

	return written, nil
}
