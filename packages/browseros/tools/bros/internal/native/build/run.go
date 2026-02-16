package build

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"slices"
	"strings"

	"bros/internal/native/common"

	"gopkg.in/yaml.v3"
)

type Options struct {
	ConfigPath  string
	Modules     string
	ListModules bool
	Setup       bool
	Prep        bool
	Build       bool
	Sign        bool
	Package     bool
	Upload      bool
	Arch        string
	BuildType   string
	ChromiumSrc string
}

type yamlConfig struct {
	Build struct {
		ChromiumSrc  string `yaml:"chromium_src"`
		Architecture string `yaml:"architecture"`
		Arch         string `yaml:"arch"`
		Type         string `yaml:"type"`
	} `yaml:"build"`
	Modules      []string `yaml:"modules"`
	RequiredEnvs []string `yaml:"required_envs"`
}

type ModuleDef struct {
	Name        string
	Description string
	Runner      func(*Context) error
}

var moduleDefs = []ModuleDef{
	{Name: "clean", Description: "Clean build artifacts and reset git state", Runner: runClean},
	{Name: "git_setup", Description: "Checkout Chromium version and sync dependencies", Runner: runGitSetup},
	{Name: "sparkle_setup", Description: "Download and setup Sparkle framework (macOS only)", Runner: runSparkleSetup},
	{Name: "configure", Description: "Configure build with GN", Runner: runConfigure},
	{Name: "patches", Description: "Apply BrowserOS patches to Chromium", Runner: runPatches},
	{Name: "series_patches", Description: "Apply series-based patches (GNU Quilt format)", Runner: runSeriesPatches},
	{Name: "chromium_replace", Description: "Replace Chromium source files with custom versions", Runner: runChromiumReplace},
	{Name: "string_replaces", Description: "Apply branding string replacements in Chromium", Runner: runStringReplaces},
	{Name: "download_resources", Description: "Download resources from Cloudflare R2", Runner: runDownloadResources},
	{Name: "resources", Description: "Copy resources (icons, binaries) into Chromium source", Runner: runResources},
	{Name: "bundled_extensions", Description: "Download and bundle extensions from CDN update manifest", Runner: runBundledExtensions},
	{Name: "compile", Description: "Build BrowserOS using autoninja", Runner: runCompile},
	{Name: "sign_macos", Description: "Sign and notarize macOS application", Runner: runSignMacOS},
	{Name: "sign_windows", Description: "Sign Windows binaries and installer", Runner: runSignWindows},
	{Name: "sign_linux", Description: "Linux code signing (no-op)", Runner: runSignLinux},
	{Name: "sparkle_sign", Description: "Sign DMGs with Sparkle Ed25519 key", Runner: runSparkleSign},
	{Name: "package_macos", Description: "Create macOS DMG package", Runner: runPackageMacOS},
	{Name: "package_windows", Description: "Create Windows installer and zip package", Runner: runPackageWindows},
	{Name: "package_linux", Description: "Create AppImage and .deb packages", Runner: runPackageLinux},
	{Name: "universal_build", Description: "Build, sign, package, upload universal binary (arm64 + x64)", Runner: runUniversalBuild},
	{Name: "upload", Description: "Upload build artifacts to Cloudflare R2", Runner: runUpload},
}

var moduleByName = func() map[string]ModuleDef {
	out := make(map[string]ModuleDef, len(moduleDefs))
	for _, m := range moduleDefs {
		out[m.Name] = m
	}
	return out
}()

var executionOrder = []struct {
	Phase   string
	Modules []string
}{
	{Phase: "setup", Modules: []string{"clean", "git_setup", "sparkle_setup"}},
	{Phase: "prep", Modules: []string{"download_resources", "resources", "bundled_extensions", "chromium_replace", "string_replaces", "patches", "configure"}},
	{Phase: "build", Modules: []string{"compile"}},
	{Phase: "sign", Modules: []string{signModuleForPlatform()}},
	{Phase: "package", Modules: []string{packageModuleForPlatform()}},
	{Phase: "upload", Modules: []string{"upload"}},
}

func Run(opts Options) error {
	if opts.ListModules {
		PrintModuleList()
		return nil
	}

	packagesDir, err := common.ResolveBrowserOSPackagesDir()
	if err != nil {
		return err
	}

	configMode := strings.TrimSpace(opts.ConfigPath) != ""
	modulesMode := strings.TrimSpace(opts.Modules) != ""
	flagsMode := opts.Setup || opts.Prep || opts.Build || opts.Sign || opts.Package || opts.Upload

	providedModes := 0
	if configMode {
		providedModes++
	}
	if modulesMode {
		providedModes++
	}
	if flagsMode {
		providedModes++
	}

	if providedModes == 0 {
		return fmt.Errorf("specify --config, --modules, or phase flags (--setup, --build, etc.)")
	}
	if providedModes > 1 {
		return fmt.Errorf("specify only one of: --config, --modules, or phase flags")
	}

	var (
		ctx          *Context
		pipeline     []string
		requiredEnvs []string
	)

	if configMode {
		if strings.TrimSpace(opts.Arch) != "" || strings.TrimSpace(opts.BuildType) != "" {
			return fmt.Errorf("cannot use --arch or --build-type with --config (YAML is authoritative)")
		}
		yamlCfg, err := readYAMLConfig(opts.ConfigPath)
		if err != nil {
			return err
		}
		ctx, err = contextFromConfig(packagesDir, opts, yamlCfg)
		if err != nil {
			return err
		}
		if len(yamlCfg.Modules) == 0 {
			return fmt.Errorf("config mode requires non-empty modules list")
		}
		pipeline = append([]string(nil), yamlCfg.Modules...)
		requiredEnvs = append([]string(nil), yamlCfg.RequiredEnvs...)
	} else {
		ctx, err = contextFromDirect(packagesDir, opts)
		if err != nil {
			return err
		}
		if modulesMode {
			pipeline = parseModules(opts.Modules)
			if len(pipeline) == 0 {
				return fmt.Errorf("--modules resolved to empty pipeline")
			}
		} else {
			pipeline = pipelineFromFlags(opts)
			if len(pipeline) == 0 {
				return fmt.Errorf("no phase flags selected")
			}
		}
	}

	if err := validateRequiredEnv(requiredEnvs); err != nil {
		return err
	}

	for _, moduleName := range pipeline {
		module, ok := moduleByName[moduleName]
		if !ok {
			return fmt.Errorf("unknown module %q", moduleName)
		}
		if err := module.Runner(ctx); err != nil {
			return fmt.Errorf("%s: %w", moduleName, err)
		}
	}

	return nil
}

func PrintModuleList() {
	fmt.Println()
	fmt.Println("======================================================================")
	fmt.Println("Available Build Modules")
	fmt.Println("======================================================================")
	fmt.Println()
	for _, m := range moduleDefs {
		fmt.Printf("  %-20s %s\n", m.Name, m.Description)
	}
	fmt.Println()
}

func contextFromConfig(packagesDir string, opts Options, cfg *yamlConfig) (*Context, error) {
	chromiumSrc := strings.TrimSpace(opts.ChromiumSrc)
	if chromiumSrc == "" {
		chromiumSrc = strings.TrimSpace(cfg.Build.ChromiumSrc)
	}
	if chromiumSrc == "" {
		return nil, fmt.Errorf("config mode requires build.chromium_src or --chromium-src")
	}

	arch := firstNonEmpty(
		strings.TrimSpace(cfg.Build.Architecture),
		strings.TrimSpace(cfg.Build.Arch),
		defaultArch(),
	)
	buildType := firstNonEmpty(strings.TrimSpace(cfg.Build.Type), "debug")

	absSrc, err := filepath.Abs(chromiumSrc)
	if err != nil {
		return nil, fmt.Errorf("resolving chromium source: %w", err)
	}
	if !dirExists(absSrc) {
		return nil, fmt.Errorf("chromium source does not exist: %s", absSrc)
	}

	return newContext(packagesDir, absSrc, arch, buildType)
}

func contextFromDirect(packagesDir string, opts Options) (*Context, error) {
	chromiumSrc := strings.TrimSpace(opts.ChromiumSrc)
	if chromiumSrc == "" {
		chromiumSrc = strings.TrimSpace(os.Getenv("CHROMIUM_SRC"))
	}
	if chromiumSrc == "" {
		return nil, fmt.Errorf("direct mode requires --chromium-src or CHROMIUM_SRC")
	}

	arch := strings.TrimSpace(opts.Arch)
	if arch == "" {
		arch = strings.TrimSpace(os.Getenv("ARCH"))
	}
	if arch == "" {
		arch = defaultArch()
	}

	buildType := strings.TrimSpace(opts.BuildType)
	if buildType == "" {
		buildType = "debug"
	}

	absSrc, err := filepath.Abs(chromiumSrc)
	if err != nil {
		return nil, fmt.Errorf("resolving chromium source: %w", err)
	}
	if !dirExists(absSrc) {
		return nil, fmt.Errorf("chromium source does not exist: %s", absSrc)
	}

	return newContext(packagesDir, absSrc, arch, buildType)
}

func readYAMLConfig(path string) (*yamlConfig, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("reading config %s: %w", path, err)
	}
	var cfg yamlConfig
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("parsing config %s: %w", path, err)
	}
	return &cfg, nil
}

func validateRequiredEnv(required []string) error {
	for _, key := range required {
		key = strings.TrimSpace(key)
		if key == "" {
			continue
		}
		if strings.TrimSpace(os.Getenv(key)) == "" {
			return fmt.Errorf("required environment variable %s is not set", key)
		}
	}
	return nil
}

func parseModules(modules string) []string {
	parts := strings.Split(modules, ",")
	out := make([]string, 0, len(parts))
	for _, p := range parts {
		if name := strings.TrimSpace(p); name != "" {
			out = append(out, name)
		}
	}
	return out
}

func pipelineFromFlags(opts Options) []string {
	var selected []string
	for _, phase := range executionOrder {
		enabled := false
		switch phase.Phase {
		case "setup":
			enabled = opts.Setup
		case "prep":
			enabled = opts.Prep
		case "build":
			enabled = opts.Build
		case "sign":
			enabled = opts.Sign
		case "package":
			enabled = opts.Package
		case "upload":
			enabled = opts.Upload
		}
		if enabled {
			selected = append(selected, phase.Modules...)
		}
	}
	return selected
}

func signModuleForPlatform() string {
	switch runtime.GOOS {
	case "darwin":
		return "sign_macos"
	case "windows":
		return "sign_windows"
	default:
		return "sign_linux"
	}
}

func packageModuleForPlatform() string {
	switch runtime.GOOS {
	case "darwin":
		return "package_macos"
	case "windows":
		return "package_windows"
	default:
		return "package_linux"
	}
}

func defaultArch() string {
	switch runtime.GOARCH {
	case "arm64":
		return "arm64"
	case "amd64":
		return "x64"
	default:
		return "x64"
	}
}

func firstNonEmpty(values ...string) string {
	for _, v := range values {
		if strings.TrimSpace(v) != "" {
			return strings.TrimSpace(v)
		}
	}
	return ""
}

func unsupportedModule(name string) func(*Context) error {
	return func(ctx *Context) error {
		return fmt.Errorf("module %q is not implemented yet in native Go migration", name)
	}
}

func containsString(slice []string, needle string) bool {
	return slices.Contains(slice, needle)
}
