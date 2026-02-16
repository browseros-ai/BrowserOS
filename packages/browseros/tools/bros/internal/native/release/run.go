package release

import (
	"context"
	"fmt"

	"bros/internal/native/r2"
)

type Options struct {
	Version     string
	List        bool
	Appcast     bool
	Publish     bool
	Download    bool
	OSFilter    string
	Output      string
	ShowModules bool
}

type ModuleInfo struct {
	Name        string
	Description string
}

var AvailableModules = []ModuleInfo{
	{Name: "list", Description: "List release artifacts from R2"},
	{Name: "appcast", Description: "Generate Sparkle appcast XML snippets"},
	{Name: "github", Description: "Create GitHub release from R2 artifacts"},
	{Name: "publish", Description: "Publish versioned artifacts to latest download URLs"},
	{Name: "download", Description: "Download release artifacts from CDN"},
}

type RunResult struct {
	Modules        []ModuleInfo
	Versions       []string
	Metadata       map[string]PlatformMetadata
	Appcast        []AppcastSnippet
	PublishResults []PublishResult
	Download       DownloadSummary
}

func ValidateOptions(opts Options) error {
	if opts.ShowModules {
		return nil
	}

	hasFlags := opts.List || opts.Appcast || opts.Publish || opts.Download
	if !hasFlags {
		return fmt.Errorf("specify a flag (--list, --appcast, --publish, --download)")
	}

	requiresVersion := opts.Appcast || opts.Publish || opts.Download
	if requiresVersion && opts.Version == "" {
		return fmt.Errorf("--version is required for this operation")
	}

	if _, err := NormalizeOSFilter(opts.OSFilter); err != nil {
		return err
	}

	return nil
}

func Run(ctx context.Context, client *r2.Client, opts Options) (RunResult, error) {
	if err := ValidateOptions(opts); err != nil {
		return RunResult{}, err
	}

	result := RunResult{}
	if opts.ShowModules {
		result.Modules = append(result.Modules, AvailableModules...)
		return result, nil
	}

	var (
		metadata   map[string]PlatformMetadata
		hasFetched bool
	)
	fetchMetadata := func() (map[string]PlatformMetadata, error) {
		if hasFetched {
			return metadata, nil
		}
		fetched, err := FetchAllReleaseMetadata(ctx, client, opts.Version)
		if err != nil {
			return nil, err
		}
		metadata = fetched
		hasFetched = true
		return metadata, nil
	}

	if opts.List {
		if opts.Version == "" {
			versions, err := ListAllVersions(ctx, client)
			if err != nil {
				return RunResult{}, err
			}
			result.Versions = versions
		} else {
			fetched, err := fetchMetadata()
			if err != nil {
				return RunResult{}, err
			}
			result.Metadata = fetched
		}
	}

	if opts.Appcast {
		fetched, err := fetchMetadata()
		if err != nil {
			return RunResult{}, err
		}
		result.Metadata = fetched

		if macOS, ok := fetched[PlatformMacOS]; ok {
			result.Appcast = GenerateAppcastSnippets(opts.Version, macOS)
		}
	}

	if opts.Publish {
		fetched, err := fetchMetadata()
		if err != nil {
			return RunResult{}, err
		}
		result.Metadata = fetched

		if len(fetched) > 0 {
			publishResults, err := PublishArtifacts(ctx, client, opts.Version, fetched, nil)
			if err != nil {
				return RunResult{}, err
			}
			result.PublishResults = publishResults
		}
	}

	if opts.Download {
		fetched, err := fetchMetadata()
		if err != nil {
			return RunResult{}, err
		}
		result.Metadata = fetched

		if len(fetched) > 0 {
			summary, err := DownloadArtifacts(ctx, opts.Version, fetched, DownloadOptions{
				OSFilter:  opts.OSFilter,
				OutputDir: opts.Output,
			})
			if err != nil {
				return RunResult{}, err
			}
			result.Download = summary
		}
	}

	return result, nil
}
