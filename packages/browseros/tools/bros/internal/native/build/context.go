package build

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"

	nativecommon "bros/internal/native/common"
)

const appBaseName = "BrowserOS"

type SparkleSignature struct {
	Signature string
	Length    int64
}

type Context struct {
	PackagesDir              string
	ChromiumSrc              string
	Architecture             string
	BuildType                string
	OutDir                   string
	ChromiumVersion          string
	BrowserOSBuildOffset     string
	BrowserOSChromiumVersion string
	SemanticVersion          string
	SparkleVersion           string
	FixedAppPath             string
	SignedApp                bool
	SparkleSignatures        map[string]SparkleSignature
	Env                      nativecommon.EnvConfig
}

func (c *Context) ConfigDir() string {
	return filepath.Join(c.PackagesDir, "build", "config")
}

func (c *Context) PatchesDir() string {
	return filepath.Join(c.PackagesDir, "chromium_patches")
}

func (c *Context) SeriesPatchesDir() string {
	return filepath.Join(c.PackagesDir, "series_patches")
}

func (c *Context) ChromiumFilesDir() string {
	return filepath.Join(c.PackagesDir, "chromium_files")
}

func (c *Context) CopyResourcesConfig() string {
	return filepath.Join(c.ConfigDir(), "copy_resources.yaml")
}

func (c *Context) DownloadResourcesConfig() string {
	return filepath.Join(c.ConfigDir(), "download_resources.yaml")
}

func (c *Context) GNFlagsPath() string {
	return filepath.Join(c.ConfigDir(), "gn", fmt.Sprintf("flags.%s.%s.gn", currentPlatformName(), c.BuildType))
}

func (c *Context) GNArgsPath() string {
	return filepath.Join(c.ChromiumSrc, c.OutDir, "args.gn")
}

func (c *Context) EntitlementsDir() string {
	return filepath.Join(c.PackagesDir, "resources", "entitlements")
}

func (c *Context) PkgDMGPath() string {
	return filepath.Join(c.ChromiumSrc, "chrome", "installer", "mac", "pkg-dmg")
}

func (c *Context) NotarizationZipPath() string {
	return filepath.Join(c.ChromiumSrc, c.OutDir, "notarize.zip")
}

func (c *Context) ChromiumAppPath() string {
	switch runtime.GOOS {
	case "darwin":
		return filepath.Join(c.ChromiumSrc, c.OutDir, "Chromium.app")
	case "windows":
		return filepath.Join(c.ChromiumSrc, c.OutDir, "chrome.exe")
	default:
		return filepath.Join(c.ChromiumSrc, c.OutDir, "chrome")
	}
}

func (c *Context) BrowserOSAppPath() string {
	if strings.TrimSpace(c.FixedAppPath) != "" {
		return c.FixedAppPath
	}

	if runtime.GOOS == "darwin" {
		universalPath := filepath.Join(c.ChromiumSrc, "out", "Default_universal", "BrowserOS.app")
		if fileExists(universalPath) {
			return universalPath
		}
		if c.BuildType == "debug" {
			debugPath := filepath.Join(c.ChromiumSrc, c.OutDir, "BrowserOS Dev.app")
			if fileExists(debugPath) {
				return debugPath
			}
		}
		return filepath.Join(c.ChromiumSrc, c.OutDir, "BrowserOS.app")
	}
	if runtime.GOOS == "windows" {
		return filepath.Join(c.ChromiumSrc, c.OutDir, "BrowserOS.exe")
	}
	return filepath.Join(c.ChromiumSrc, c.OutDir, strings.ToLower(appBaseName))
}

func (c *Context) SparkleDir() string {
	return filepath.Join(c.ChromiumSrc, "third_party", "sparkle")
}

func (c *Context) SparkleURL() string {
	return fmt.Sprintf("https://github.com/sparkle-project/Sparkle/releases/download/%s/Sparkle-%s.tar.xz", c.SparkleVersion, c.SparkleVersion)
}

func (c *Context) DistDir() string {
	return filepath.Join(c.PackagesDir, "releases", c.SemanticVersion)
}

func (c *Context) ReleasePath(platform string) string {
	return fmt.Sprintf("releases/%s/%s/", c.SemanticVersion, platform)
}

func (c *Context) ArtifactName(artifactType string) (string, error) {
	if strings.TrimSpace(c.SemanticVersion) == "" {
		return "", fmt.Errorf("semantic version is not set")
	}

	version := c.SemanticVersion
	arch := strings.TrimSpace(c.Architecture)
	if arch == "" {
		arch = defaultArch()
	}

	switch artifactType {
	case "dmg":
		return fmt.Sprintf("%s_v%s_%s.dmg", appBaseName, version, arch), nil
	case "appimage":
		return fmt.Sprintf("%s_v%s_%s.AppImage", appBaseName, version, arch), nil
	case "deb":
		debArch := arch
		if arch == "x64" {
			debArch = "amd64"
		}
		return fmt.Sprintf("%s_v%s_%s.deb", appBaseName, version, debArch), nil
	case "installer":
		return fmt.Sprintf("%s_v%s_%s_installer.exe", appBaseName, version, arch), nil
	case "installer_zip":
		return fmt.Sprintf("%s_v%s_%s_installer.zip", appBaseName, version, arch), nil
	default:
		return "", fmt.Errorf("unknown artifact type %q", artifactType)
	}
}

func newContext(packagesDir, chromiumSrc, arch, buildType string) (*Context, error) {
	chromiumVersion, versionParts, err := loadChromiumVersion(packagesDir)
	if err != nil {
		return nil, err
	}
	buildOffset, err := loadBuildOffset(packagesDir)
	if err != nil {
		return nil, err
	}
	semanticVersion, err := loadSemanticVersion(packagesDir)
	if err != nil {
		return nil, err
	}

	outDir := filepath.Join("out", "Default_"+arch)

	browserOSChromiumVersion := ""
	if len(versionParts) == 4 && buildOffset != "" {
		baseBuild, err := strconv.Atoi(versionParts[2])
		if err != nil {
			return nil, fmt.Errorf("parsing chromium BUILD value %q: %w", versionParts[2], err)
		}
		offsetInt, err := strconv.Atoi(buildOffset)
		if err != nil {
			return nil, fmt.Errorf("parsing BROWSEROS_BUILD_OFFSET %q: %w", buildOffset, err)
		}
		browserOSChromiumVersion = fmt.Sprintf("%s.%s.%d.%s", versionParts[0], versionParts[1], baseBuild+offsetInt, versionParts[3])
	}

	sparkleVersion := ""
	if browserOSChromiumVersion != "" {
		parts := strings.Split(browserOSChromiumVersion, ".")
		if len(parts) >= 4 {
			sparkleVersion = fmt.Sprintf("%s.%s", parts[2], parts[3])
		}
	}

	return &Context{
		PackagesDir:              packagesDir,
		ChromiumSrc:              chromiumSrc,
		Architecture:             arch,
		BuildType:                buildType,
		OutDir:                   outDir,
		ChromiumVersion:          chromiumVersion,
		BrowserOSBuildOffset:     buildOffset,
		BrowserOSChromiumVersion: browserOSChromiumVersion,
		SemanticVersion:          semanticVersion,
		SparkleVersion:           defaultString(sparkleVersion, "0.0"),
		SparkleSignatures:        map[string]SparkleSignature{},
		Env:                      nativecommon.LoadEnv(packagesDir),
	}, nil
}

func loadChromiumVersion(packagesDir string) (string, []string, error) {
	path := filepath.Join(packagesDir, "CHROMIUM_VERSION")
	data, err := os.ReadFile(path)
	if err != nil {
		return "", nil, fmt.Errorf("reading CHROMIUM_VERSION: %w", err)
	}

	var major, minor, build, patch string
	for _, line := range strings.Split(strings.TrimSpace(string(data)), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || !strings.Contains(line, "=") {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		key := strings.TrimSpace(parts[0])
		val := strings.TrimSpace(parts[1])
		switch key {
		case "MAJOR":
			major = val
		case "MINOR":
			minor = val
		case "BUILD":
			build = val
		case "PATCH":
			patch = val
		}
	}
	if major == "" || minor == "" || build == "" || patch == "" {
		return "", nil, fmt.Errorf("invalid CHROMIUM_VERSION format in %s", path)
	}
	return fmt.Sprintf("%s.%s.%s.%s", major, minor, build, patch), []string{major, minor, build, patch}, nil
}

func loadBuildOffset(packagesDir string) (string, error) {
	path := filepath.Join(packagesDir, "build", "config", "BROWSEROS_BUILD_OFFSET")
	data, err := os.ReadFile(path)
	if err != nil {
		return "", fmt.Errorf("reading BROWSEROS_BUILD_OFFSET: %w", err)
	}
	return strings.TrimSpace(string(data)), nil
}

func loadSemanticVersion(packagesDir string) (string, error) {
	path := filepath.Join(packagesDir, "resources", "BROWSEROS_VERSION")
	data, err := os.ReadFile(path)
	if err != nil {
		return "", fmt.Errorf("reading resources/BROWSEROS_VERSION: %w", err)
	}

	values := map[string]string{}
	for _, line := range strings.Split(strings.TrimSpace(string(data)), "\n") {
		line = strings.TrimSpace(line)
		if line == "" || !strings.Contains(line, "=") {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		values[strings.TrimSpace(parts[0])] = strings.TrimSpace(parts[1])
	}

	major := defaultString(values["BROWSEROS_MAJOR"], "0")
	minor := defaultString(values["BROWSEROS_MINOR"], "0")
	build := defaultString(values["BROWSEROS_BUILD"], "0")
	patch := defaultString(values["BROWSEROS_PATCH"], "0")

	if patch != "0" {
		return fmt.Sprintf("%s.%s.%s.%s", major, minor, build, patch), nil
	}
	if build != "0" {
		return fmt.Sprintf("%s.%s.%s", major, minor, build), nil
	}
	return fmt.Sprintf("%s.%s.0", major, minor), nil
}

func defaultString(value, fallback string) string {
	if strings.TrimSpace(value) == "" {
		return fallback
	}
	return value
}

func fileExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir()
}

func dirExists(path string) bool {
	info, err := os.Stat(path)
	return err == nil && info.IsDir()
}

func currentPlatformName() string {
	switch runtime.GOOS {
	case "darwin":
		return "macos"
	case "windows":
		return "windows"
	default:
		return "linux"
	}
}
