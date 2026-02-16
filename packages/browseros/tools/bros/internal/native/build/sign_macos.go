package build

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"
)

type macBinarySignInfo struct {
	IdentifierSuffix string
	Options          string
	Entitlements     string
}

var browserOSServerSignInfo = map[string]macBinarySignInfo{
	"browseros_server": {
		IdentifierSuffix: "browseros_server",
		Options:          "runtime",
		Entitlements:     "browseros-executable-entitlements.plist",
	},
	"codex": {
		IdentifierSuffix: "codex",
		Options:          "runtime",
		Entitlements:     "browseros-executable-entitlements.plist",
	},
}

type macComponents struct {
	Helpers     []string
	XPCServices []string
	Frameworks  []string
	Dylibs      []string
	Executables []string
	Apps        []string
}

func runSignMacOS(ctx *Context) error {
	if runtime.GOOS != "darwin" {
		return fmt.Errorf("macOS signing requires macOS")
	}

	appPath := ctx.BrowserOSAppPath()
	if !dirExists(appPath) {
		return fmt.Errorf("app not found at: %s", appPath)
	}

	if err := validateMacOSSigningEnv(ctx); err != nil {
		return err
	}

	unlockMacOSKeychain(ctx)

	if err := runCmd("", "xattr", "-cs", appPath); err != nil {
		return fmt.Errorf("clearing extended attributes: %w", err)
	}

	if err := signAllMacOSComponents(ctx, appPath, ctx.Env.MacOSCertificateName); err != nil {
		return err
	}

	if err := verifyMacOSSignature(appPath); err != nil {
		return err
	}

	if err := notarizeMacOSApp(ctx, appPath); err != nil {
		return err
	}

	ctx.SignedApp = true
	return nil
}

func validateMacOSSigningEnv(ctx *Context) error {
	missing := make([]string, 0)
	if strings.TrimSpace(ctx.Env.MacOSCertificateName) == "" {
		missing = append(missing, "MACOS_CERTIFICATE_NAME")
	}
	if strings.TrimSpace(ctx.Env.MacOSNotarizationAppleID) == "" {
		missing = append(missing, "PROD_MACOS_NOTARIZATION_APPLE_ID")
	}
	if strings.TrimSpace(ctx.Env.MacOSNotarizationTeamID) == "" {
		missing = append(missing, "PROD_MACOS_NOTARIZATION_TEAM_ID")
	}
	if strings.TrimSpace(ctx.Env.MacOSNotarizationPassword) == "" {
		missing = append(missing, "PROD_MACOS_NOTARIZATION_PWD")
	}
	if len(missing) > 0 {
		return fmt.Errorf("missing environment variables: %s", strings.Join(missing, ", "))
	}
	return nil
}

func unlockMacOSKeychain(ctx *Context) {
	password := strings.TrimSpace(ctx.Env.MacOSKeychainPassword)
	if password == "" {
		return
	}

	keychainPath := filepath.Join(os.Getenv("HOME"), "Library", "Keychains", "login.keychain-db")
	if !fileExists(keychainPath) {
		return
	}

	_, _ = runCmdCapture("", "security", "unlock-keychain", "-p", password, keychainPath)
	_, _ = runCmdCapture("", "security", "set-keychain-settings", "-t", "3600", keychainPath)
}

func signAllMacOSComponents(ctx *Context, appPath, certificate string) error {
	components := discoverMacOSComponents(ctx, appPath)

	for _, xpc := range components.XPCServices {
		if err := signMacOSComponent(certificate, xpc, macIdentifierForPath(xpc), macSigningOptionsForPath(xpc), ""); err != nil {
			return err
		}
	}

	for _, nestedApp := range components.Apps {
		if err := signMacOSComponent(certificate, nestedApp, macIdentifierForPath(nestedApp), macSigningOptionsForPath(nestedApp), ""); err != nil {
			return err
		}
	}

	for _, exePath := range components.Executables {
		entitlements := ""
		if info, ok := browserOSServerBinaryInfo(exePath); ok {
			entitlements = filepath.Join(ctx.EntitlementsDir(), info.Entitlements)
		}
		if err := signMacOSComponent(certificate, exePath, macIdentifierForPath(exePath), macSigningOptionsForPath(exePath), entitlements); err != nil {
			return err
		}
	}

	for _, dylib := range components.Dylibs {
		if err := signMacOSComponent(certificate, dylib, macIdentifierForPath(dylib), "", ""); err != nil {
			return err
		}
	}

	for _, helper := range components.Helpers {
		entitlements := ""
		switch {
		case strings.Contains(helper, "Renderer"):
			entitlements = filepath.Join(ctx.EntitlementsDir(), "helper-renderer-entitlements.plist")
		case strings.Contains(helper, "GPU"):
			entitlements = filepath.Join(ctx.EntitlementsDir(), "helper-gpu-entitlements.plist")
		case strings.Contains(helper, "Plugin"):
			entitlements = filepath.Join(ctx.EntitlementsDir(), "helper-plugin-entitlements.plist")
		}
		if err := signMacOSComponent(certificate, helper, macIdentifierForPath(helper), macSigningOptionsForPath(helper), entitlements); err != nil {
			return err
		}
	}

	frameworks := append([]string(nil), components.Frameworks...)
	sort.Slice(frameworks, func(i, j int) bool {
		iSparkle := strings.Contains(frameworks[i], "Sparkle")
		jSparkle := strings.Contains(frameworks[j], "Sparkle")
		if iSparkle == jSparkle {
			return frameworks[i] < frameworks[j]
		}
		return iSparkle && !jSparkle
	})
	for _, framework := range frameworks {
		if err := signMacOSComponent(certificate, framework, macIdentifierForPath(framework), "", ""); err != nil {
			return err
		}
	}

	mainExecutable, err := findMainMacExecutable(appPath)
	if err != nil {
		return err
	}
	if err := signMacOSComponent(certificate, mainExecutable, "com.browseros.BrowserOS", "", ""); err != nil {
		return err
	}

	requirements := `=designated => identifier "com.browseros.BrowserOS" and anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */`
	appEntitlements := firstExistingPath([]string{
		filepath.Join(ctx.EntitlementsDir(), "app-entitlements.plist"),
		filepath.Join(ctx.EntitlementsDir(), "app-entitlements-chrome.plist"),
		filepath.Join(ctx.PackagesDir, "entitlements", "app-entitlements.plist"),
		filepath.Join(ctx.PackagesDir, "entitlements", "app-entitlements-chrome.plist"),
		filepath.Join(ctx.PackagesDir, "build", "src", "chrome", "app", "app-entitlements.plist"),
		filepath.Join(ctx.PackagesDir, "build", "src", "chrome", "app", "app-entitlements-chrome.plist"),
		filepath.Join(ctx.ChromiumSrc, "chrome", "app", "app-entitlements.plist"),
		filepath.Join(ctx.ChromiumSrc, "chrome", "app", "app-entitlements-chrome.plist"),
	})

	cmd := []string{
		"--sign", certificate,
		"--force",
		"--timestamp",
		"--identifier", "com.browseros.BrowserOS",
		"--options", "restrict,library,runtime,kill",
		"--requirements", requirements,
	}
	if appEntitlements != "" {
		cmd = append(cmd, "--entitlements", appEntitlements)
	}
	cmd = append(cmd, appPath)
	if err := runCmd("", "codesign", cmd...); err != nil {
		return fmt.Errorf("signing app bundle: %w", err)
	}

	return nil
}

func signMacOSComponent(certificate, componentPath, identifier, options, entitlements string) error {
	args := []string{"--sign", certificate, "--force", "--timestamp"}
	if strings.TrimSpace(identifier) != "" {
		args = append(args, "--identifier", identifier)
	}
	if strings.TrimSpace(options) != "" {
		args = append(args, "--options", options)
	}
	if strings.TrimSpace(entitlements) != "" && fileExists(entitlements) {
		args = append(args, "--entitlements", entitlements)
	}
	args = append(args, componentPath)
	if err := runCmd("", "codesign", args...); err != nil {
		return fmt.Errorf("signing %s: %w", componentPath, err)
	}
	return nil
}

func verifyMacOSSignature(appPath string) error {
	result, err := runCmdCapture("", "codesign", "--verify", "--deep", "--strict", "--verbose=2", appPath)
	if err != nil {
		return err
	}
	if result.ExitCode != 0 {
		return fmt.Errorf("signature verification failed: %s", firstNonEmpty(result.Stderr, result.Stdout))
	}
	return nil
}

func notarizeMacOSApp(ctx *Context, appPath string) error {
	zipPath := ctx.NotarizationZipPath()
	_ = os.Remove(zipPath)

	if err := runCmd("", "ditto", "-c", "-k", "--keepParent", appPath, zipPath); err != nil {
		return fmt.Errorf("creating notarization archive: %w", err)
	}

	storeCredRes, err := runCmdCapture(
		"",
		"xcrun", "notarytool", "store-credentials", "notarytool-profile",
		"--apple-id", ctx.Env.MacOSNotarizationAppleID,
		"--team-id", ctx.Env.MacOSNotarizationTeamID,
		"--password", ctx.Env.MacOSNotarizationPassword,
	)
	if err != nil {
		return err
	}

	submitArgs := []string{"notarytool", "submit", zipPath, "--wait"}
	if storeCredRes.ExitCode == 0 {
		submitArgs = append(submitArgs, "--keychain-profile", "notarytool-profile")
	} else {
		submitArgs = append(submitArgs,
			"--apple-id", ctx.Env.MacOSNotarizationAppleID,
			"--team-id", ctx.Env.MacOSNotarizationTeamID,
			"--password", ctx.Env.MacOSNotarizationPassword,
		)
	}

	submitRes, err := runCmdCapture("", "xcrun", submitArgs...)
	if err != nil {
		return err
	}
	if submitRes.ExitCode != 0 {
		return fmt.Errorf("notarization submit failed: %s", firstNonEmpty(submitRes.Stderr, submitRes.Stdout))
	}
	if !strings.Contains(strings.ToLower(submitRes.Stdout), "status: accepted") {
		return fmt.Errorf("notarization failed, output: %s", submitRes.Stdout)
	}

	stapleRes, err := runCmdCapture("", "xcrun", "stapler", "staple", appPath)
	if err != nil {
		return err
	}
	if stapleRes.ExitCode != 0 {
		return fmt.Errorf("failed to staple notarization ticket: %s", firstNonEmpty(stapleRes.Stderr, stapleRes.Stdout))
	}

	spctlRes, err := runCmdCapture("", "spctl", "-a", "-vvv", appPath)
	if err != nil {
		return err
	}
	if spctlRes.ExitCode != 0 {
		return fmt.Errorf("gatekeeper check failed: %s", firstNonEmpty(spctlRes.Stderr, spctlRes.Stdout))
	}

	validateRes, err := runCmdCapture("", "xcrun", "stapler", "validate", appPath)
	if err != nil {
		return err
	}
	if validateRes.ExitCode != 0 {
		return fmt.Errorf("stapler validation failed: %s", firstNonEmpty(validateRes.Stderr, validateRes.Stdout))
	}

	_ = os.Remove(zipPath)
	return nil
}

func discoverMacOSComponents(ctx *Context, appPath string) macComponents {
	components := macComponents{}
	frameworkPath := filepath.Join(appPath, "Contents", "Frameworks")
	if !dirExists(frameworkPath) {
		return components
	}

	browserOSFrameworkRoots := make([]string, 0)
	frameworkNames := []string{"BrowserOS Framework.framework", "BrowserOS Dev Framework.framework"}
	for _, fwName := range frameworkNames {
		fwPath := filepath.Join(frameworkPath, fwName)
		if !dirExists(fwPath) {
			continue
		}
		versioned := filepath.Join(fwPath, "Versions", ctx.BrowserOSChromiumVersion)
		if strings.TrimSpace(ctx.BrowserOSChromiumVersion) != "" && dirExists(versioned) {
			browserOSFrameworkRoots = append(browserOSFrameworkRoots, versioned)
		}
		browserOSFrameworkRoots = append(browserOSFrameworkRoots, fwPath)
	}

	for _, fwRoot := range browserOSFrameworkRoots {
		helpersDir := filepath.Join(fwRoot, "Helpers")
		if !dirExists(helpersDir) {
			continue
		}
		entries, err := os.ReadDir(helpersDir)
		if err != nil {
			continue
		}
		for _, entry := range entries {
			fullPath := filepath.Join(helpersDir, entry.Name())
			if entry.IsDir() && strings.HasSuffix(entry.Name(), ".app") {
				components.Helpers = append(components.Helpers, fullPath)
				continue
			}
			if entry.Type().IsRegular() && filepath.Ext(entry.Name()) == "" && isExecutableFile(fullPath) {
				components.Executables = append(components.Executables, fullPath)
			}
		}
		break
	}

	_ = filepath.WalkDir(frameworkPath, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if d.IsDir() {
			if strings.HasSuffix(d.Name(), ".xpc") {
				components.XPCServices = append(components.XPCServices, path)
			}
			if strings.HasSuffix(d.Name(), ".framework") {
				components.Frameworks = append(components.Frameworks, path)
				if strings.Contains(path, "Sparkle.framework") {
					autoupdate := filepath.Join(path, "Versions", "B", "Autoupdate")
					if fileExists(autoupdate) {
						components.Executables = append(components.Executables, autoupdate)
					}
				}
			}
			if strings.HasSuffix(d.Name(), ".app") {
				components.Apps = append(components.Apps, path)
			}
			return nil
		}
		if strings.HasSuffix(d.Name(), ".dylib") {
			components.Dylibs = append(components.Dylibs, path)
		}
		return nil
	})

	for _, fwRoot := range browserOSFrameworkRoots {
		librariesDir := filepath.Join(fwRoot, "Libraries")
		entries, err := os.ReadDir(librariesDir)
		if err != nil {
			continue
		}
		for _, entry := range entries {
			if entry.IsDir() || filepath.Ext(entry.Name()) != ".dylib" {
				continue
			}
			components.Dylibs = append(components.Dylibs, filepath.Join(librariesDir, entry.Name()))
		}
	}

	serverDir := filepath.Join(appPath, "Contents", "Resources", "BrowserOSServer")
	if dirExists(serverDir) {
		_ = filepath.WalkDir(serverDir, func(path string, d os.DirEntry, err error) error {
			if err != nil || d.IsDir() {
				return nil
			}
			if filepath.Ext(d.Name()) == "" && isExecutableFile(path) {
				components.Executables = append(components.Executables, path)
			}
			return nil
		})
	}

	helperSet := toSet(components.Helpers)
	filteredApps := make([]string, 0, len(components.Apps))
	for _, nestedApp := range components.Apps {
		if _, ok := helperSet[nestedApp]; ok {
			continue
		}
		filteredApps = append(filteredApps, nestedApp)
	}
	components.Apps = filteredApps

	components.Helpers = dedupeSorted(components.Helpers)
	components.XPCServices = dedupeSorted(components.XPCServices)
	components.Frameworks = dedupeSorted(components.Frameworks)
	components.Dylibs = dedupeSorted(components.Dylibs)
	components.Executables = dedupeSorted(components.Executables)
	components.Apps = dedupeSorted(components.Apps)
	return components
}

func macIdentifierForPath(componentPath string) string {
	baseIdentifier := "com.browseros"
	name := strings.TrimSuffix(filepath.Base(componentPath), filepath.Ext(componentPath))

	specialIdentifiers := map[string]string{
		"Downloader":              "org.sparkle-project.Downloader",
		"Installer":               "org.sparkle-project.Installer",
		"Updater":                 "org.sparkle-project.Updater",
		"Autoupdate":              "org.sparkle-project.Autoupdate",
		"Sparkle":                 "org.sparkle-project.Sparkle",
		"chrome_crashpad_handler": baseIdentifier + ".crashpad_handler",
		"app_mode_loader":         baseIdentifier + ".app_mode_loader",
		"web_app_shortcut_copier": baseIdentifier + ".web_app_shortcut_copier",
	}
	for key, identifier := range specialIdentifiers {
		if strings.Contains(componentPath, key) {
			return identifier
		}
	}

	if info, ok := browserOSServerBinaryInfo(componentPath); ok {
		return fmt.Sprintf("%s.%s", baseIdentifier, info.IdentifierSuffix)
	}

	if strings.Contains(name, "Helper") {
		start := strings.Index(name, "(")
		end := strings.Index(name, ")")
		if start >= 0 && end > start {
			helperType := strings.ToLower(strings.TrimSpace(name[start+1 : end]))
			return fmt.Sprintf("%s.helper.%s", baseIdentifier, helperType)
		}
		return baseIdentifier + ".helper"
	}

	if filepath.Ext(componentPath) == ".framework" {
		if name == "BrowserOS Framework" || name == "BrowserOS Dev Framework" {
			return baseIdentifier + ".framework"
		}
		return fmt.Sprintf("%s.%s", baseIdentifier, strings.ToLower(strings.ReplaceAll(name, " ", "_")))
	}

	if filepath.Ext(componentPath) == ".dylib" {
		return fmt.Sprintf("%s.%s", baseIdentifier, name)
	}

	return fmt.Sprintf("%s.%s", baseIdentifier, strings.ToLower(strings.ReplaceAll(name, " ", "_")))
}

func macSigningOptionsForPath(componentPath string) string {
	name := filepath.Base(componentPath)
	lowerPath := strings.ToLower(componentPath)

	if strings.Contains(lowerPath, "sparkle") {
		return "runtime"
	}
	if strings.Contains(name, "Helper (Renderer)") || strings.Contains(name, "Helper (GPU)") || strings.Contains(name, "Helper (Plugin)") {
		return "restrict,kill,runtime"
	}
	if info, ok := browserOSServerBinaryInfo(componentPath); ok {
		if strings.TrimSpace(info.Options) != "" {
			return info.Options
		}
	}
	if filepath.Ext(componentPath) == ".dylib" {
		return "restrict,library,runtime,kill"
	}
	return "runtime"
}

func browserOSServerBinaryInfo(componentPath string) (macBinarySignInfo, bool) {
	name := strings.ToLower(strings.TrimSuffix(filepath.Base(componentPath), filepath.Ext(componentPath)))
	info, ok := browserOSServerSignInfo[name]
	return info, ok
}

func findMainMacExecutable(appPath string) (string, error) {
	candidates := []string{
		filepath.Join(appPath, "Contents", "MacOS", "BrowserOS"),
		filepath.Join(appPath, "Contents", "MacOS", "BrowserOS Dev"),
	}
	for _, candidate := range candidates {
		if fileExists(candidate) {
			return candidate, nil
		}
	}
	return "", fmt.Errorf("main executable not found in %s", filepath.Join(appPath, "Contents", "MacOS"))
}

func firstExistingPath(candidates []string) string {
	for _, candidate := range candidates {
		if fileExists(candidate) {
			return candidate
		}
	}
	return ""
}

func dedupeSorted(values []string) []string {
	set := make(map[string]struct{}, len(values))
	for _, value := range values {
		value = strings.TrimSpace(value)
		if value == "" {
			continue
		}
		set[value] = struct{}{}
	}
	out := make([]string, 0, len(set))
	for value := range set {
		out = append(out, value)
	}
	sort.Strings(out)
	return out
}

func toSet(values []string) map[string]struct{} {
	set := make(map[string]struct{}, len(values))
	for _, value := range values {
		set[value] = struct{}{}
	}
	return set
}

func isExecutableFile(path string) bool {
	info, err := os.Stat(path)
	if err != nil {
		return false
	}
	return !info.IsDir() && info.Mode()&0o111 != 0
}
