package release

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"strings"

	"bros/internal/native/r2"
)

func ListAllVersions(ctx context.Context, client *r2.Client) ([]string, error) {
	return client.ListReleaseVersions(ctx)
}

func FetchAllReleaseMetadata(ctx context.Context, client *r2.Client, version string) (map[string]PlatformMetadata, error) {
	metadata := make(map[string]PlatformMetadata)
	for _, platform := range Platforms {
		releaseData, err := FetchReleaseMetadataForPlatform(ctx, client, version, platform)
		if err != nil {
			if errors.Is(err, r2.ErrNotFound) {
				continue
			}
			return nil, err
		}
		metadata[platform] = releaseData
	}
	return metadata, nil
}

func FetchReleaseMetadataForPlatform(ctx context.Context, client *r2.Client, version, platform string) (PlatformMetadata, error) {
	data, err := client.FetchReleaseJSON(ctx, version, platform)
	if err != nil {
		return PlatformMetadata{}, err
	}
	return ParseReleaseJSON(data)
}

func ParseReleaseJSON(data []byte) (PlatformMetadata, error) {
	var release PlatformMetadata
	if err := json.Unmarshal(data, &release); err != nil {
		return PlatformMetadata{}, fmt.Errorf("parse release.json: %w", err)
	}
	if release.Artifacts == nil {
		release.Artifacts = map[string]Artifact{}
	}
	return release, nil
}

func FormatSize(sizeBytes int64) string {
	const (
		kb = int64(1024)
		mb = kb * 1024
		gb = mb * 1024
	)

	switch {
	case sizeBytes >= gb:
		return fmt.Sprintf("%.1f GB", float64(sizeBytes)/float64(gb))
	case sizeBytes >= mb:
		return fmt.Sprintf("%.0f MB", float64(sizeBytes)/float64(mb))
	case sizeBytes >= kb:
		return fmt.Sprintf("%.0f KB", float64(sizeBytes)/float64(kb))
	default:
		return fmt.Sprintf("%d B", sizeBytes)
	}
}

func NormalizeOSFilter(value string) (string, error) {
	if strings.TrimSpace(value) == "" {
		return "", nil
	}

	normalized := OSNameMap[strings.ToLower(strings.TrimSpace(value))]
	if normalized == "" {
		return "", fmt.Errorf("invalid --os value: %s", value)
	}
	return normalized, nil
}
