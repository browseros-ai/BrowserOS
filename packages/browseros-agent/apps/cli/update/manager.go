package update

import (
	"context"
	"fmt"
	"net/http"
	"os"
	"runtime"
	"time"
)

const (
	DefaultManifestURL     = "https://cdn.browseros.com/cli/latest/manifest.json"
	DefaultCheckTTL        = 24 * time.Hour
	DefaultHTTPTimeout     = 2 * time.Second
	DefaultDownloadTimeout = 5 * time.Minute
	SkipCheckEnv           = "BROWSEROS_SKIP_UPDATE_CHECK"
	InstallMethodEnv       = "BROWSEROS_INSTALL_METHOD"
)

type Options struct {
	CurrentVersion  string
	ManifestURL     string
	CheckTTL        time.Duration
	HTTPTimeout     time.Duration
	DownloadTimeout time.Duration
	JSONOutput      bool
	Debug           bool
	Automatic       bool
	HTTPClient      *http.Client
	Now             func() time.Time
}

type Manager struct {
	options Options
	state   *State
}

type CheckResult struct {
	CurrentVersion    string    `json:"current_version"`
	LatestVersion     string    `json:"latest_version"`
	LatestPublishedAt string    `json:"latest_published_at,omitempty"`
	UpdateAvailable   bool      `json:"update_available"`
	CheckedAt         time.Time `json:"checked_at"`
	Asset             *Asset    `json:"asset,omitempty"`
}

func NewManager(options Options) *Manager {
	if options.ManifestURL == "" {
		options.ManifestURL = DefaultManifestURL
	}
	if options.CheckTTL == 0 {
		options.CheckTTL = DefaultCheckTTL
	}
	if options.HTTPTimeout == 0 {
		options.HTTPTimeout = DefaultHTTPTimeout
	}
	if options.DownloadTimeout == 0 {
		options.DownloadTimeout = DefaultDownloadTimeout
	}
	if options.Now == nil {
		options.Now = time.Now
	}
	if options.HTTPClient == nil {
		options.HTTPClient = &http.Client{}
	}

	state, err := LoadState()
	if err != nil {
		state = &State{}
	}

	return &Manager{
		options: options,
		state:   state,
	}
}

func (m *Manager) CachedNotice() string {
	if !m.AutomaticEnabled() || m.state == nil || m.state.LatestVersion == "" {
		return ""
	}

	comparison, err := CompareVersions(m.options.CurrentVersion, m.state.LatestVersion)
	if err != nil || comparison >= 0 {
		return ""
	}

	return FormatNotice(m.options.CurrentVersion, m.state.LatestVersion)
}

func (m *Manager) AutomaticEnabled() bool {
	if !m.options.Automatic || m.options.JSONOutput {
		return false
	}
	if os.Getenv(SkipCheckEnv) != "" {
		return false
	}
	if installedViaPackageManager() {
		return false
	}
	return IsReleaseVersion(m.options.CurrentVersion)
}

func installedViaPackageManager() bool {
	method := os.Getenv(InstallMethodEnv)
	switch method {
	case "npm", "brew", "homebrew":
		return true
	}
	return false
}

func (m *Manager) ShouldCheck() bool {
	if !m.AutomaticEnabled() {
		return false
	}
	return m.state.IsStale(m.options.Now(), m.options.CheckTTL)
}

func (m *Manager) StartBackgroundCheck(ctx context.Context) <-chan struct{} {
	done := make(chan struct{})
	if !m.ShouldCheck() {
		close(done)
		return done
	}

	go func() {
		defer close(done)
		_, _ = m.CheckNow(ctx)
	}()

	return done
}

func (m *Manager) CheckNow(ctx context.Context) (*CheckResult, error) {
	if !IsReleaseVersion(m.options.CurrentVersion) {
		return nil, fmt.Errorf("self-update is unavailable for non-release build %q", m.options.CurrentVersion)
	}

	checkCtx, cancel := context.WithTimeout(ctx, m.options.HTTPTimeout)
	defer cancel()

	manifest, err := FetchManifest(checkCtx, cloneHTTPClient(m.options.HTTPClient), m.options.ManifestURL)
	if err != nil {
		m.recordError(err)
		return nil, err
	}

	asset, err := SelectAsset(manifest, runtime.GOOS, runtime.GOARCH)
	if err != nil {
		m.recordError(err)
		return nil, err
	}

	comparison, err := CompareVersions(m.options.CurrentVersion, manifest.Version)
	if err != nil {
		m.recordError(err)
		return nil, err
	}

	result := &CheckResult{
		CurrentVersion:    m.options.CurrentVersion,
		LatestVersion:     manifest.Version,
		LatestPublishedAt: manifest.PublishedAt,
		UpdateAvailable:   comparison < 0,
		CheckedAt:         m.options.Now(),
	}
	if result.UpdateAvailable {
		assetCopy := asset
		result.Asset = &assetCopy
	}

	m.state = &State{
		LastCheckedAt:     result.CheckedAt,
		LatestVersion:     manifest.Version,
		LatestPublishedAt: manifest.PublishedAt,
		AssetURL:          asset.URL,
	}
	_ = SaveState(m.state)

	return result, nil
}

func (m *Manager) Apply(ctx context.Context, result *CheckResult) error {
	if result == nil || !result.UpdateAvailable || result.Asset == nil {
		return fmt.Errorf("browseros-cli is already up to date")
	}

	downloadCtx, cancel := context.WithTimeout(ctx, m.options.DownloadTimeout)
	defer cancel()

	archive, err := DownloadAsset(downloadCtx, cloneHTTPClient(m.options.HTTPClient), *result.Asset)
	if err != nil {
		return err
	}
	if err := VerifyChecksum(archive, result.Asset.SHA256); err != nil {
		return err
	}
	binary, err := ExtractBinary(archive, result.Asset.ArchiveFormat)
	if err != nil {
		return err
	}

	targetPath, err := os.Executable()
	if err != nil {
		return err
	}
	if err := CheckPermissions(targetPath); err != nil {
		return fmt.Errorf(
			"cannot replace %s: %w\n\nReinstall with the installer script or move the binary to a writable location.",
			targetPath,
			err,
		)
	}
	if err := ApplyBinary(binary, targetPath); err != nil {
		return err
	}

	m.saveAppliedState(result)

	return nil
}

func FormatNotice(currentVersion, latestVersion string) string {
	notice := fmt.Sprintf(
		"Update available: browseros-cli v%s (current v%s)",
		latestVersion,
		currentVersion,
	)

	switch os.Getenv(InstallMethodEnv) {
	case "npm":
		notice += "\nRun `npm update -g browseros-cli` to upgrade."
	case "brew", "homebrew":
		notice += "\nRun `brew upgrade browseros-cli` to upgrade."
	default:
		notice += "\nRun `browseros-cli update` to upgrade."
	}

	return notice
}

func (m *Manager) recordError(err error) {
	state := &State{}
	if m.state != nil {
		*state = *m.state
	}
	state.CheckError = err.Error()
	m.state = state
	_ = SaveState(state)
}

func (m *Manager) saveAppliedState(result *CheckResult) {
	state := &State{
		LastCheckedAt:     m.options.Now(),
		LatestVersion:     result.LatestVersion,
		LatestPublishedAt: result.LatestPublishedAt,
		AssetURL:          result.Asset.URL,
	}
	m.state = state
	_ = SaveState(state)
}

func cloneHTTPClient(client *http.Client) *http.Client {
	if client == nil {
		return &http.Client{}
	}

	cloned := *client
	cloned.Timeout = 0
	return &cloned
}
