package release

import (
	"context"
	"fmt"
	"sort"

	"bros/internal/native/r2"
)

type PublishOperation struct {
	Platform       string
	ArtifactKey    string
	Filename       string
	SourceKey      string
	DestinationKey string
}

type PublishResult struct {
	PublishOperation
	Err error
}

func BuildPublishOperations(version string, metadata map[string]PlatformMetadata, platforms []string) []PublishOperation {
	resolvedPlatforms := platforms
	if len(resolvedPlatforms) == 0 {
		resolvedPlatforms = Platforms
	}

	operations := make([]PublishOperation, 0)
	for _, platform := range resolvedPlatforms {
		releaseData, ok := metadata[platform]
		if !ok {
			continue
		}

		mapping, ok := DownloadPathMapping[platform]
		if !ok {
			continue
		}

		artifactKeys := make([]string, 0, len(releaseData.Artifacts))
		for key := range releaseData.Artifacts {
			artifactKeys = append(artifactKeys, key)
		}
		sort.Strings(artifactKeys)

		for _, artifactKey := range artifactKeys {
			destination, mapped := mapping[artifactKey]
			if !mapped {
				continue
			}
			artifact := releaseData.Artifacts[artifactKey]
			operations = append(operations, PublishOperation{
				Platform:       platform,
				ArtifactKey:    artifactKey,
				Filename:       artifact.Filename,
				SourceKey:      fmt.Sprintf("releases/%s/%s/%s", version, platform, artifact.Filename),
				DestinationKey: destination,
			})
		}
	}

	return operations
}

func PublishArtifacts(ctx context.Context, client *r2.Client, version string, metadata map[string]PlatformMetadata, platforms []string) ([]PublishResult, error) {
	if len(metadata) == 0 {
		return nil, fmt.Errorf("no release metadata found for version %s", version)
	}

	ops := BuildPublishOperations(version, metadata, platforms)
	results := make([]PublishResult, 0, len(ops))
	for _, op := range ops {
		err := client.CopyObject(ctx, op.SourceKey, op.DestinationKey)
		results = append(results, PublishResult{
			PublishOperation: op,
			Err:              err,
		})
	}
	return results, nil
}
