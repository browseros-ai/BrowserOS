package release

import (
	"context"
	"errors"
	"fmt"
	"net/http"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"bros/internal/native/r2"
)

var ErrGitHubReleaseAlreadyExists = errors.New("github release already exists")

type CommandRunner interface {
	Run(ctx context.Context, name string, args ...string) (stdout string, stderr string, err error)
}

type ExecCommandRunner struct{}

func (ExecCommandRunner) Run(ctx context.Context, name string, args ...string) (string, string, error) {
	cmd := commandContext(ctx, name, args...)
	stdout, stderr, err := runCommand(cmd)
	return stdout, stderr, err
}

type GitHubUploadResult struct {
	Platform    string
	ArtifactKey string
	Filename    string
	Err         error
}

type GitHubReleaseOptions struct {
	Version    string
	Repo       string
	Draft      bool
	SkipUpload bool
	Title      string
	Runner     CommandRunner
	HTTPClient *http.Client
}

type GitHubReleaseResult struct {
	TagVersion     string
	Repo           string
	ReleaseURL     string
	ReleaseExisted bool
	UploadResults  []GitHubUploadResult
	Appcast        []AppcastSnippet
}

func CheckGHCLI(ctx context.Context, runner CommandRunner) error {
	if runner == nil {
		runner = ExecCommandRunner{}
	}
	_, stderr, err := runner.Run(ctx, "gh", "--version")
	if err != nil {
		if strings.TrimSpace(stderr) != "" {
			return fmt.Errorf("gh CLI not available: %s", strings.TrimSpace(stderr))
		}
		return fmt.Errorf("gh CLI not available: %w", err)
	}
	return nil
}

func GetRepoFromGit(ctx context.Context, runner CommandRunner) (string, error) {
	if runner == nil {
		runner = ExecCommandRunner{}
	}
	stdout, stderr, err := runner.Run(ctx, "git", "remote", "get-url", "origin")
	if err != nil {
		if strings.TrimSpace(stderr) != "" {
			return "", fmt.Errorf("git remote get-url origin failed: %s", strings.TrimSpace(stderr))
		}
		return "", err
	}

	repo, ok := ParseGitHubRepo(strings.TrimSpace(stdout))
	if !ok {
		return "", fmt.Errorf("could not detect GitHub repo from origin remote")
	}
	return repo, nil
}

func ParseGitHubRepo(remoteURL string) (string, bool) {
	trimmed := strings.TrimSpace(remoteURL)
	if trimmed == "" {
		return "", false
	}

	if !strings.Contains(trimmed, "github.com") {
		return "", false
	}

	if strings.HasPrefix(trimmed, "git@") {
		parts := strings.SplitN(trimmed, ":", 2)
		if len(parts) != 2 {
			return "", false
		}
		return strings.TrimSuffix(parts[1], ".git"), true
	}

	parts := strings.Split(trimmed, "/")
	if len(parts) < 2 {
		return "", false
	}
	repo := strings.TrimSuffix(strings.Join(parts[len(parts)-2:], "/"), ".git")
	if strings.Count(repo, "/") != 1 {
		return "", false
	}
	return repo, true
}

func NormalizeVersion(version string) string {
	parts := strings.Split(version, ".")
	if len(parts) >= 3 {
		return strings.Join(parts[:3], ".")
	}
	return version
}

func GenerateReleaseNotes(version string, metadata map[string]PlatformMetadata) string {
	chromiumVersion := "unknown"
	for _, platform := range Platforms {
		releaseData, ok := metadata[platform]
		if !ok {
			continue
		}
		if releaseData.ChromiumVersion != "" {
			chromiumVersion = releaseData.ChromiumVersion
			break
		}
	}

	var b strings.Builder
	b.WriteString(fmt.Sprintf("## BrowserOS v%s\n\n", version))
	b.WriteString(fmt.Sprintf("Chromium version: %s\n\n", chromiumVersion))
	b.WriteString("### Downloads\n\n")

	for _, platform := range Platforms {
		releaseData, ok := metadata[platform]
		if !ok {
			continue
		}

		b.WriteString(fmt.Sprintf("**%s:**\n", PlatformDisplayNames[platform]))
		artifactKeys := make([]string, 0, len(releaseData.Artifacts))
		for key := range releaseData.Artifacts {
			artifactKeys = append(artifactKeys, key)
		}
		sort.Strings(artifactKeys)

		for _, key := range artifactKeys {
			artifact := releaseData.Artifacts[key]
			b.WriteString(fmt.Sprintf("- [%s](%s)\n", artifact.Filename, artifact.URL))
		}
		b.WriteString("\n")
	}

	return b.String()
}

func CreateGitHubRelease(ctx context.Context, runner CommandRunner, version, repo, title, notes string, draft bool) (string, error) {
	if runner == nil {
		runner = ExecCommandRunner{}
	}

	args := []string{
		"release",
		"create",
		"v" + version,
		"--repo", repo,
		"--title", title,
		"--notes", notes,
	}
	if draft {
		args = append(args, "--draft")
	}

	stdout, stderr, err := runner.Run(ctx, "gh", args...)
	if err != nil {
		combined := strings.ToLower(stdout + "\n" + stderr + "\n" + err.Error())
		if strings.Contains(combined, "already exists") {
			return "", ErrGitHubReleaseAlreadyExists
		}
		if strings.TrimSpace(stderr) != "" {
			return "", fmt.Errorf("gh release create failed: %s", strings.TrimSpace(stderr))
		}
		return "", err
	}

	return strings.TrimSpace(stdout), nil
}

func UploadToGitHubRelease(ctx context.Context, runner CommandRunner, version, repo string, filePath string) error {
	if runner == nil {
		runner = ExecCommandRunner{}
	}

	_, stderr, err := runner.Run(ctx, "gh", "release", "upload", "v"+version, filePath, "--repo", repo)
	if err != nil {
		if strings.TrimSpace(stderr) != "" {
			return fmt.Errorf("gh release upload failed: %s", strings.TrimSpace(stderr))
		}
		return err
	}
	return nil
}

func DownloadAndUploadArtifacts(ctx context.Context, runner CommandRunner, httpClient *http.Client, version, repo string, metadata map[string]PlatformMetadata, platforms []string) ([]GitHubUploadResult, error) {
	if runner == nil {
		runner = ExecCommandRunner{}
	}
	if httpClient == nil {
		httpClient = &http.Client{Timeout: 30 * time.Second}
	}
	if len(platforms) == 0 {
		platforms = Platforms
	}

	tmpDir, err := os.MkdirTemp("", "bros-release-*")
	if err != nil {
		return nil, err
	}
	defer os.RemoveAll(tmpDir)

	results := make([]GitHubUploadResult, 0)
	failed := 0

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

		for _, key := range artifactKeys {
			artifact := releaseData.Artifacts[key]
			result := GitHubUploadResult{Platform: platform, ArtifactKey: key, Filename: artifact.Filename}
			if artifact.URL == "" || artifact.Filename == "" {
				result.Err = fmt.Errorf("artifact missing url or filename")
				results = append(results, result)
				failed++
				continue
			}

			destination := filepath.Join(tmpDir, artifact.Filename)
			if _, err := downloadFile(ctx, httpClient, artifact.URL, destination); err != nil {
				result.Err = err
				results = append(results, result)
				failed++
				continue
			}

			if err := UploadToGitHubRelease(ctx, runner, version, repo, destination); err != nil {
				result.Err = err
				failed++
			}
			results = append(results, result)
		}
	}

	if failed > 0 {
		return results, fmt.Errorf("%d artifact upload(s) failed", failed)
	}
	return results, nil
}

func CreateAndUploadGitHubRelease(ctx context.Context, client *r2.Client, opts GitHubReleaseOptions) (GitHubReleaseResult, error) {
	runner := opts.Runner
	if runner == nil {
		runner = ExecCommandRunner{}
	}

	if err := CheckGHCLI(ctx, runner); err != nil {
		return GitHubReleaseResult{}, err
	}

	repo := strings.TrimSpace(opts.Repo)
	if repo == "" {
		detectedRepo, err := GetRepoFromGit(ctx, runner)
		if err != nil {
			return GitHubReleaseResult{}, err
		}
		repo = detectedRepo
	}

	metadata, err := FetchAllReleaseMetadata(ctx, client, opts.Version)
	if err != nil {
		return GitHubReleaseResult{}, err
	}
	if len(metadata) == 0 {
		return GitHubReleaseResult{}, fmt.Errorf("no release metadata found for version %s", opts.Version)
	}

	tagVersion := NormalizeVersion(opts.Version)
	releaseTitle := opts.Title
	if strings.TrimSpace(releaseTitle) == "" {
		releaseTitle = "v" + tagVersion
	}
	notes := GenerateReleaseNotes(tagVersion, metadata)

	releaseURL, createErr := CreateGitHubRelease(ctx, runner, tagVersion, repo, releaseTitle, notes, opts.Draft)
	releaseExisted := false
	if createErr != nil {
		if errors.Is(createErr, ErrGitHubReleaseAlreadyExists) {
			releaseExisted = true
		} else {
			return GitHubReleaseResult{}, createErr
		}
	}

	result := GitHubReleaseResult{
		TagVersion:     tagVersion,
		Repo:           repo,
		ReleaseURL:     releaseURL,
		ReleaseExisted: releaseExisted,
	}

	if !opts.SkipUpload {
		uploadResults, err := DownloadAndUploadArtifacts(ctx, runner, opts.HTTPClient, tagVersion, repo, metadata, nil)
		result.UploadResults = uploadResults
		if err != nil {
			return result, err
		}
	}

	if macOS, ok := metadata[PlatformMacOS]; ok {
		result.Appcast = GenerateAppcastSnippets(tagVersion, macOS)
	}

	return result, nil
}
