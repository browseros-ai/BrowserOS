// Package sign ports build/modules/sign: macOS codesign+notarize, Windows
// eSigner, Linux no-op, and Sparkle DMG signing.
package sign

import (
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	"github.com/browseros-ai/BrowserOS/packages/browseros/build_go/internal/buildctx"
	"github.com/browseros-ai/BrowserOS/packages/browseros/build_go/internal/envx"
	"github.com/browseros-ai/BrowserOS/packages/browseros/build_go/internal/execx"
	"github.com/browseros-ai/BrowserOS/packages/browseros/build_go/internal/logx"
	"github.com/browseros-ai/BrowserOS/packages/browseros/build_go/internal/serverbin"
)

func run(ctx *buildctx.Context, args ...string) (execx.Result, error) {
	return execx.Checked(ctx.Runner, execx.Cmd{Args: args, Dir: ctx.ChromiumSrc, Stream: logx.Out})
}

func runUnchecked(ctx *buildctx.Context, args ...string) execx.Result {
	res, _ := ctx.Runner.Run(execx.Cmd{Args: args, Dir: ctx.ChromiumSrc, Stream: logx.Out})
	return res
}

const (
	serverResourcesSourceRel = "chrome/browser/browseros/server/resources"
	serverResourcesBundleRel = "Contents/Resources/BrowserOSServer/default/resources"
)

// VerifyServerResourcesBundle checks the app bundle ships exactly what the
// build staged for the server (macos.py verify_server_resources_bundle):
// path + exec-bit comparison only. Returns problem strings; empty = OK.
func VerifyServerResourcesBundle(appPath, chromiumSrc string) []string {
	sourceRoot := filepath.Join(chromiumSrc, filepath.FromSlash(serverResourcesSourceRel))
	bundleRoot := filepath.Join(appPath, filepath.FromSlash(serverResourcesBundleRel))

	if info, err := os.Stat(sourceRoot); err != nil || !info.IsDir() {
		logx.Warning(fmt.Sprintf(
			"Staged server resources not found at %s - skipping bundle verification", sourceRoot))
		return nil
	}

	var problems []string
	staged := map[string]bool{}
	filepath.WalkDir(sourceRoot, func(path string, d os.DirEntry, err error) error {
		if err != nil || d.IsDir() || d.Name() == ".DS_Store" {
			return nil
		}
		rel, err := filepath.Rel(sourceRoot, path)
		if err != nil {
			return nil
		}
		staged[filepath.ToSlash(rel)] = true
		bundleFile := filepath.Join(bundleRoot, rel)
		bundleInfo, err := os.Stat(bundleFile)
		if err != nil || bundleInfo.IsDir() {
			problems = append(problems, "missing from app bundle: "+filepath.ToSlash(rel))
			return nil
		}
		sourceInfo, err := os.Stat(path)
		if err == nil && sourceInfo.Mode()&0o111 != 0 && bundleInfo.Mode()&0o111 == 0 {
			problems = append(problems, "lost executable bit in app bundle: "+filepath.ToSlash(rel))
		}
		return nil
	})

	if info, err := os.Stat(bundleRoot); err == nil && info.IsDir() {
		filepath.WalkDir(bundleRoot, func(path string, d os.DirEntry, err error) error {
			if err != nil || d.IsDir() || d.Name() == ".DS_Store" {
				return nil
			}
			rel, err := filepath.Rel(bundleRoot, path)
			if err != nil {
				return nil
			}
			if !staged[filepath.ToSlash(rel)] {
				logx.Warning(fmt.Sprintf(
					"App bundle has server file not in staged resources (stale?): %s", filepath.ToSlash(rel)))
			}
			return nil
		})
	}
	sort.Strings(problems)
	return problems
}

// UnlockKeychain unlocks the login keychain for non-interactive sessions
// (macos.py unlock_keychain). Failures only warn.
func UnlockKeychain(ctx *buildctx.Context) {
	password := envx.MacOSKeychainPassword()
	if password == "" {
		logx.Warning("MACOS_KEYCHAIN_PASSWORD not set — keychain may be locked (will fail over SSH)")
		return
	}
	home, err := os.UserHomeDir()
	if err != nil {
		return
	}
	keychain := filepath.Join(home, "Library", "Keychains", "login.keychain-db")
	if _, err := os.Stat(keychain); err != nil {
		logx.Warning("Keychain not found at " + keychain)
		return
	}
	logx.Info("🔓 Unlocking login keychain...")
	runUnchecked(ctx, "security", "unlock-keychain", "-p", password, keychain)
	// Prevent auto-lock during long signing + notarization runs.
	runUnchecked(ctx, "security", "set-keychain-settings", "-t", "3600", keychain)
}

// CheckSigningEnvironment reports whether macOS signing env is complete
// (macos.py check_signing_environment).
func CheckSigningEnvironment() bool {
	var missing []string
	if envx.MacOSCertificateName() == "" {
		missing = append(missing, "MACOS_CERTIFICATE_NAME")
	}
	if envx.MacOSNotarizationAppleID() == "" {
		missing = append(missing, "PROD_MACOS_NOTARIZATION_APPLE_ID")
	}
	if envx.MacOSNotarizationTeamID() == "" {
		missing = append(missing, "PROD_MACOS_NOTARIZATION_TEAM_ID")
	}
	if envx.MacOSNotarizationPassword() == "" {
		missing = append(missing, "PROD_MACOS_NOTARIZATION_PWD")
	}
	if len(missing) > 0 {
		logx.Error("❌ Signing requires macOS environment variables!")
		logx.Error("Missing environment variables: " + strings.Join(missing, ", "))
		return false
	}
	return true
}

// Components groups everything discovered for signing
// (macos.py find_components_to_sign). Lists are sorted for determinism.
type Components struct {
	Helpers     []string
	XPCServices []string
	Frameworks  []string
	Dylibs      []string
	Executables []string
	Apps        []string
}

func globRecursive(root, suffix string) []string {
	var matches []string
	filepath.WalkDir(root, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil
		}
		if strings.HasSuffix(d.Name(), suffix) {
			matches = append(matches, path)
			if d.IsDir() {
				return filepath.SkipDir // don't descend into bundles for same-suffix nesting
			}
		}
		return nil
	})
	sort.Strings(matches)
	return matches
}

func isExecutableFile(path string) bool {
	info, err := os.Stat(path)
	return err == nil && !info.IsDir() && info.Mode()&0o111 != 0
}

// FindComponentsToSign discovers all signable pieces of the bundle.
func FindComponentsToSign(ctx *buildctx.Context, appPath string) Components {
	var c Components
	frameworksDir := filepath.Join(appPath, "Contents", "Frameworks")

	// BrowserOS Framework (release/debug names), preferring versioned paths.
	var browserOSFrameworks []string
	for _, name := range []string{"BrowserOS Framework.framework", "BrowserOS Dev Framework.framework"} {
		fwPath := filepath.Join(frameworksDir, name)
		if _, err := os.Stat(fwPath); err != nil {
			continue
		}
		browserOSFrameworks = append(browserOSFrameworks, fwPath)
		if ctx.BrowserOSChromiumVersion != "" {
			versioned := filepath.Join(fwPath, "Versions", ctx.BrowserOSChromiumVersion)
			if _, err := os.Stat(versioned); err == nil {
				browserOSFrameworks = append([]string{versioned}, browserOSFrameworks...)
			}
		}
	}

	// Helper apps + bare helper executables (first valid framework path wins).
	for _, fwPath := range browserOSFrameworks {
		helpersDir := filepath.Join(fwPath, "Helpers")
		entries, err := os.ReadDir(helpersDir)
		if err != nil {
			continue
		}
		for _, entry := range entries {
			full := filepath.Join(helpersDir, entry.Name())
			if strings.HasSuffix(entry.Name(), ".app") {
				c.Helpers = append(c.Helpers, full)
			} else if !entry.IsDir() && filepath.Ext(entry.Name()) == "" && isExecutableFile(full) {
				c.Executables = append(c.Executables, full)
			}
		}
		break
	}

	c.XPCServices = globRecursive(frameworksDir, ".xpc")
	c.Frameworks = globRecursive(frameworksDir, ".framework")

	// Sparkle ships a versioned Autoupdate executable.
	for _, fwPath := range c.Frameworks {
		if strings.Contains(fwPath, "Sparkle.framework") {
			autoupdate := filepath.Join(fwPath, "Versions", "B", "Autoupdate")
			if isExecutableFile(autoupdate) {
				c.Executables = append(c.Executables, autoupdate)
			}
		}
	}

	// Dylibs: BrowserOS Framework Libraries first, then any others.
	seenDylib := map[string]bool{}
	for _, fwPath := range browserOSFrameworks {
		librariesDir := filepath.Join(fwPath, "Libraries")
		for _, dylib := range globRecursive(librariesDir, ".dylib") {
			if !seenDylib[dylib] {
				seenDylib[dylib] = true
				c.Dylibs = append(c.Dylibs, dylib)
			}
		}
	}
	for _, dylib := range globRecursive(frameworksDir, ".dylib") {
		if !seenDylib[dylib] {
			seenDylib[dylib] = true
			c.Dylibs = append(c.Dylibs, dylib)
		}
	}

	// Nested apps (e.g. Sparkle's Updater.app) that aren't helpers.
	helperSet := map[string]bool{}
	for _, helper := range c.Helpers {
		helperSet[helper] = true
	}
	for _, app := range globRecursive(frameworksDir, ".app") {
		if !helperSet[app] {
			c.Apps = append(c.Apps, app)
		}
	}

	// BrowserOS Server binaries under Contents/Resources/BrowserOSServer.
	serverDir := filepath.Join(appPath, "Contents", "Resources", "BrowserOSServer")
	if _, err := os.Stat(serverDir); err == nil {
		filepath.WalkDir(serverDir, func(path string, d os.DirEntry, err error) error {
			if err != nil || d.IsDir() {
				return nil
			}
			if filepath.Ext(d.Name()) != "" || !isExecutableFile(path) {
				return nil
			}
			if _, ok := serverbin.MacOSSignSpecFor(path); ok {
				c.Executables = append(c.Executables, path)
			}
			return nil
		})
	}
	sort.Strings(c.Executables)
	return c
}

const baseIdentifier = "com.browseros"

// IdentifierForComponent generates the codesign identifier
// (macos.py get_identifier_for_component).
func IdentifierForComponent(componentPath string) string {
	name := strings.TrimSuffix(filepath.Base(componentPath), filepath.Ext(componentPath))

	specialIdentifiers := []struct{ key, id string }{
		{"Downloader", "org.sparkle-project.Downloader"},
		{"Installer", "org.sparkle-project.Installer"},
		{"Updater", "org.sparkle-project.Updater"},
		{"Autoupdate", "org.sparkle-project.Autoupdate"},
		{"Sparkle", "org.sparkle-project.Sparkle"},
		{"chrome_crashpad_handler", baseIdentifier + ".crashpad_handler"},
		{"app_mode_loader", baseIdentifier + ".app_mode_loader"},
		{"web_app_shortcut_copier", baseIdentifier + ".web_app_shortcut_copier"},
	}
	for _, special := range specialIdentifiers {
		if strings.Contains(componentPath, special.key) {
			return special.id
		}
	}

	if spec, ok := serverbin.MacOSSignSpecFor(componentPath); ok {
		return baseIdentifier + "." + spec.IdentifierSuffix
	}

	if strings.Contains(name, "Helper") {
		if open := strings.Index(name, "("); open >= 0 {
			if close := strings.Index(name, ")"); close > open {
				return baseIdentifier + ".helper." + strings.ToLower(name[open+1:close])
			}
		}
		return baseIdentifier + ".helper"
	}

	if strings.HasSuffix(componentPath, ".framework") {
		if name == "BrowserOS Framework" || name == "BrowserOS Dev Framework" {
			return baseIdentifier + ".framework"
		}
		return baseIdentifier + "." + strings.ToLower(strings.ReplaceAll(name, " ", "_"))
	}
	if strings.HasSuffix(componentPath, ".dylib") {
		return baseIdentifier + "." + name
	}
	return baseIdentifier + "." + strings.ToLower(strings.ReplaceAll(name, " ", "_"))
}

// SigningOptions picks --options per component type
// (macos.py get_signing_options).
func SigningOptions(componentPath string) string {
	name := filepath.Base(componentPath)
	if strings.Contains(strings.ToLower(componentPath), "sparkle") {
		return "runtime"
	}
	if strings.Contains(name, "Helper (Renderer)") ||
		strings.Contains(name, "Helper (GPU)") ||
		strings.Contains(name, "Helper (Plugin)") {
		return "restrict,kill,runtime"
	}
	if spec, ok := serverbin.MacOSSignSpecFor(componentPath); ok {
		return spec.Options
	}
	if strings.HasSuffix(componentPath, ".dylib") {
		return "restrict,library,runtime,kill"
	}
	return "runtime"
}

// runProbe runs a read-only Mach-O inspection quietly (no Stream → execx
// captures without logging).
func runProbe(ctx *buildctx.Context, args ...string) execx.Result {
	res, err := ctx.Runner.Run(execx.Cmd{Args: args, Dir: ctx.ChromiumSrc})
	if err != nil {
		logx.Warning(fmt.Sprintf("Mach-O probe failed to run (%s): %v", args[0], err))
		return execx.Result{Code: 1}
	}
	return res
}

// machoArchs returns the architectures lipo reports for a file; nil when it
// is not Mach-O.
func machoArchs(ctx *buildctx.Context, path string) []string {
	res := runProbe(ctx, "lipo", "-archs", path)
	if res.Code != 0 {
		return nil
	}
	return strings.Fields(res.Stdout)
}

// sliceHasEmbeddedInfoPlist reports whether the given slice carries a
// __TEXT,__info_plist section.
func sliceHasEmbeddedInfoPlist(ctx *buildctx.Context, path, arch string) bool {
	res := runProbe(ctx, "otool", "-arch", arch, "-l", path)
	return res.Code == 0 && strings.Contains(res.Stdout, "sectname __info_plist")
}

// asymmetricInfoPlistArchs returns the archs of a fat file whose slices
// disagree on an embedded Info.plist (macos.py
// find_asymmetric_info_plist_archs). codesign, signing a fat file, binds the
// file-level Info.plist into every slice's CodeDirectory — a slice without
// the section then never validates and Apple's notary rejects it (the
// upstream claude binary ships the section on arm64 only). nil = thin,
// symmetric, or not Mach-O.
func asymmetricInfoPlistArchs(ctx *buildctx.Context, path string) []string {
	st, err := os.Lstat(path)
	if err != nil || !st.Mode().IsRegular() {
		return nil
	}
	archs := machoArchs(ctx, path)
	if len(archs) < 2 {
		return nil
	}
	withPlist := 0
	for _, arch := range archs {
		if sliceHasEmbeddedInfoPlist(ctx, path, arch) {
			withPlist++
		}
	}
	if withPlist == 0 || withPlist == len(archs) {
		return nil
	}
	return archs
}

func codesignCmd(componentPath, certificate, identifier, options, entitlements string) []string {
	cmd := []string{"codesign", "--sign", certificate, "--force", "--timestamp"}
	if identifier != "" {
		cmd = append(cmd, "--identifier", identifier)
	}
	if options != "" {
		cmd = append(cmd, "--options", options)
	}
	if entitlements != "" {
		if _, err := os.Stat(entitlements); err == nil {
			cmd = append(cmd, "--entitlements", entitlements)
		}
	}
	return append(cmd, componentPath)
}

// signFatComponentPerSlice signs each slice as a thin file and lipos them
// back together (macos.py sign_fat_component_per_slice).
func signFatComponentPerSlice(ctx *buildctx.Context, componentPath, certificate string, archs []string, identifier, options, entitlements string) error {
	st, err := os.Stat(componentPath)
	if err != nil {
		return err
	}
	tmpDir, err := os.MkdirTemp(filepath.Dir(componentPath), ".sign-slices-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(tmpDir)

	name := filepath.Base(componentPath)
	thins := make([]string, 0, len(archs))
	for _, arch := range archs {
		thin := filepath.Join(tmpDir, name+"."+arch)
		if _, err := run(ctx, "lipo", componentPath, "-thin", arch, "-output", thin); err != nil {
			return fmt.Errorf("failed to extract %s slice of %s: %w", arch, componentPath, err)
		}
		if _, err := run(ctx, codesignCmd(thin, certificate, identifier, options, entitlements)...); err != nil {
			return fmt.Errorf("failed to sign %s slice of %s: %w", arch, componentPath, err)
		}
		thins = append(thins, thin)
	}

	fat := filepath.Join(tmpDir, name+".fat")
	args := append([]string{"lipo", "-create"}, thins...)
	args = append(args, "-output", fat)
	if _, err := run(ctx, args...); err != nil {
		return fmt.Errorf("failed to reassemble %s: %w", componentPath, err)
	}
	if err := os.Chmod(fat, st.Mode().Perm()); err != nil {
		return err
	}
	return os.Rename(fat, componentPath)
}

func signComponent(ctx *buildctx.Context, componentPath, certificate, identifier, options, entitlements string) error {
	if archs := asymmetricInfoPlistArchs(ctx, componentPath); len(archs) > 0 {
		logx.Warning(fmt.Sprintf(
			"%s: slices disagree on embedded Info.plist (%s) — signing per-slice",
			filepath.Base(componentPath), strings.Join(archs, ", ")))
		return signFatComponentPerSlice(ctx, componentPath, certificate, archs, identifier, options, entitlements)
	}
	if _, err := run(ctx, codesignCmd(componentPath, certificate, identifier, options, entitlements)...); err != nil {
		return fmt.Errorf("failed to sign %s: %w", componentPath, err)
	}
	return nil
}

func serverEntitlementsPath(ctx *buildctx.Context, componentPath string) string {
	spec, ok := serverbin.MacOSSignSpecFor(componentPath)
	if !ok || spec.Entitlements == "" {
		return ""
	}
	candidate := filepath.Join(ctx.EntitlementsDir(), spec.Entitlements)
	if _, err := os.Stat(candidate); err == nil {
		return candidate
	}
	return ""
}

// SignAllComponents signs the bundle bottom-up (macos.py
// sign_all_components): XPC → nested apps → executables → dylibs → helpers →
// frameworks (Sparkle first) → main executable → app bundle.
func SignAllComponents(ctx *buildctx.Context, appPath, certificate string) error {
	logx.Info("🔍 Discovering components to sign...")
	c := FindComponentsToSign(ctx, appPath)
	total := len(c.Helpers) + len(c.XPCServices) + len(c.Frameworks) + len(c.Dylibs) + len(c.Executables) + len(c.Apps)
	logx.Info(fmt.Sprintf("Found %d components to sign:", total))

	logx.Info("\n🔏 Signing XPC Services...")
	for _, xpc := range c.XPCServices {
		if err := signComponent(ctx, xpc, certificate, IdentifierForComponent(xpc), SigningOptions(xpc), ""); err != nil {
			return err
		}
	}

	if len(c.Apps) > 0 {
		logx.Info("\n🔏 Signing nested applications...")
		for _, nested := range c.Apps {
			if err := signComponent(ctx, nested, certificate, IdentifierForComponent(nested), SigningOptions(nested), ""); err != nil {
				return err
			}
		}
	}

	if len(c.Executables) > 0 {
		logx.Info("\n🔏 Signing executables...")
		for _, exe := range c.Executables {
			if err := signComponent(ctx, exe, certificate, IdentifierForComponent(exe), SigningOptions(exe), serverEntitlementsPath(ctx, exe)); err != nil {
				return err
			}
		}
	}

	if len(c.Dylibs) > 0 {
		logx.Info("\n🔏 Signing dynamic libraries...")
		for _, dylib := range c.Dylibs {
			if err := signComponent(ctx, dylib, certificate, IdentifierForComponent(dylib), "", ""); err != nil {
				return err
			}
		}
	}

	if len(c.Helpers) > 0 {
		logx.Info("\n🔏 Signing helper applications...")
		for _, helper := range c.Helpers {
			entitlements := ""
			name := filepath.Base(helper)
			var entitlementsName string
			switch {
			case strings.Contains(name, "Renderer"):
				entitlementsName = "helper-renderer-entitlements.plist"
			case strings.Contains(name, "GPU"):
				entitlementsName = "helper-gpu-entitlements.plist"
			case strings.Contains(name, "Plugin"):
				entitlementsName = "helper-plugin-entitlements.plist"
			}
			if entitlementsName != "" {
				candidate := filepath.Join(ctx.EntitlementsDir(), entitlementsName)
				if _, err := os.Stat(candidate); err == nil {
					entitlements = candidate
				}
			}
			if err := signComponent(ctx, helper, certificate, IdentifierForComponent(helper), SigningOptions(helper), entitlements); err != nil {
				return err
			}
		}
	}

	if len(c.Frameworks) > 0 {
		logx.Info("\n🔏 Signing frameworks...")
		frameworks := append([]string(nil), c.Frameworks...)
		sort.SliceStable(frameworks, func(i, j int) bool {
			return strings.Contains(filepath.Base(frameworks[i]), "Sparkle") &&
				!strings.Contains(filepath.Base(frameworks[j]), "Sparkle")
		})
		for _, framework := range frameworks {
			if err := signComponent(ctx, framework, certificate, IdentifierForComponent(framework), "", ""); err != nil {
				return err
			}
		}
	}

	logx.Info("\n🔏 Signing main executable...")
	var mainExe string
	for _, exeName := range []string{"BrowserOS", "BrowserOS Dev"} {
		candidate := filepath.Join(appPath, "Contents", "MacOS", exeName)
		if _, err := os.Stat(candidate); err == nil {
			mainExe = candidate
			break
		}
	}
	if mainExe == "" {
		return fmt.Errorf("main executable not found in %s", filepath.Join(appPath, "Contents", "MacOS"))
	}
	if err := signComponent(ctx, mainExe, certificate, "com.browseros.BrowserOS", "", ""); err != nil {
		return err
	}

	logx.Info("\n🔏 Signing application bundle...")
	requirements := `=designated => identifier "com.browseros.BrowserOS" and ` +
		"anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] /* exists */ and " +
		"certificate leaf[field.1.2.840.113635.100.6.1.13] /* exists */"

	entitlements := findAppEntitlements(ctx, appPath)
	cmd := []string{
		"codesign", "--sign", certificate, "--force", "--timestamp",
		"--identifier", "com.browseros.BrowserOS",
		"--options", "restrict,library,runtime,kill",
		"--requirements", requirements,
	}
	if entitlements != "" {
		logx.Info("  Using entitlements: " + entitlements)
		cmd = append(cmd, "--entitlements", entitlements)
	} else {
		logx.Warning("No app entitlements file found, signing without entitlements")
	}
	cmd = append(cmd, appPath)
	if _, err := run(ctx, cmd...); err != nil {
		return err
	}
	return nil
}

func findAppEntitlements(ctx *buildctx.Context, appPath string) string {
	dirs := []string{
		ctx.EntitlementsDir(),
		filepath.Join(ctx.RootDir, "entitlements"),
		filepath.Join(ctx.RootDir, "build", "src", "chrome", "app"),
		filepath.Join(filepath.Dir(filepath.Dir(filepath.Dir(appPath))), "chrome", "app"),
	}
	for _, name := range []string{"app-entitlements.plist", "app-entitlements-chrome.plist"} {
		for _, dir := range dirs {
			candidate := filepath.Join(dir, name)
			if _, err := os.Stat(candidate); err == nil {
				return candidate
			}
		}
	}
	return ""
}

// VerifySignature runs deep strict verification (macos.py verify_signature).
func VerifySignature(ctx *buildctx.Context, appPath string) error {
	logx.Info("\n🔍 Verifying application signature integrity...")
	res := runUnchecked(ctx, "codesign", "--verify", "--deep", "--strict", "--verbose=2", appPath)
	if res.Code != 0 {
		return fmt.Errorf("signature verification failed")
	}

	// --deep seals plain executables under Resources/ as files without
	// validating their own signatures (Apple's notary does, per slice) —
	// verify each file-type component directly so a bad slice fails here
	// instead of after a multi-minute notarization round-trip. Helpers,
	// frameworks, and XPC services are proper sub-bundles --deep already
	// recurses into.
	comps := FindComponentsToSign(ctx, appPath)
	for _, comp := range append(append([]string{}, comps.Executables...), comps.Dylibs...) {
		res := runUnchecked(ctx, "codesign", "--verify", "--verbose=2", comp)
		if res.Code != 0 {
			return fmt.Errorf("component signature verification failed: %s", comp)
		}
	}

	logx.Success("Signature verification passed")
	return nil
}

// NotarizeApp zips, submits, staples, and verifies notarization
// (macos.py notarize_app).
func NotarizeApp(ctx *buildctx.Context, appPath string) error {
	logx.Info("\n📤 Preparing for notarization...")
	notarizeZip := ctx.NotarizationZip()
	os.Remove(notarizeZip)

	if _, err := run(ctx, "ditto", "-c", "-k", "--keepParent", appPath, notarizeZip); err != nil {
		return err
	}
	logx.Success("Archive created for notarization")

	logx.Info("🔑 Storing notarization credentials...")
	storeRes := runUnchecked(ctx, "xcrun", "notarytool", "store-credentials", "notarytool-profile",
		"--apple-id", envx.MacOSNotarizationAppleID(),
		"--team-id", envx.MacOSNotarizationTeamID(),
		"--password", envx.MacOSNotarizationPassword())

	logx.Info("📤 Submitting application for notarization (this may take a while)...")
	var submitArgs []string
	if storeRes.Code == 0 {
		submitArgs = []string{"xcrun", "notarytool", "submit", notarizeZip,
			"--keychain-profile", "notarytool-profile", "--wait"}
	} else {
		logx.Warning("Keychain profile unavailable — passing credentials directly")
		submitArgs = []string{"xcrun", "notarytool", "submit", notarizeZip,
			"--apple-id", envx.MacOSNotarizationAppleID(),
			"--team-id", envx.MacOSNotarizationTeamID(),
			"--password", envx.MacOSNotarizationPassword(), "--wait"}
	}
	submitRes := runUnchecked(ctx, submitArgs...)
	if submitRes.Code != 0 {
		return fmt.Errorf("notarization submission failed")
	}
	if !strings.Contains(submitRes.Stdout, "status: Accepted") {
		for _, line := range strings.Split(submitRes.Stdout, "\n") {
			if strings.Contains(line, "id:") {
				id := strings.Fields(strings.TrimSpace(strings.SplitN(line, "id:", 2)[1]))
				if len(id) > 0 {
					logx.Info(fmt.Sprintf(
						`Get detailed logs with: xcrun notarytool log %s --keychain-profile "notarytool-profile"`, id[0]))
				}
				break
			}
		}
		return fmt.Errorf("app notarization failed - status was not 'Accepted'")
	}
	logx.Success("App notarization successful - status: Accepted")

	logx.Info("📎 Stapling notarization ticket to application...")
	if res := runUnchecked(ctx, "xcrun", "stapler", "staple", appPath); res.Code != 0 {
		return fmt.Errorf("failed to staple notarization ticket")
	}
	logx.Success("Notarization ticket stapled successfully")

	os.Remove(notarizeZip)

	logx.Info("\n🔍 Verifying notarization status...")
	if res := runUnchecked(ctx, "spctl", "-a", "-vvv", appPath); res.Code != 0 {
		return fmt.Errorf("gatekeeper check failed")
	}
	if res := runUnchecked(ctx, "xcrun", "stapler", "validate", appPath); res.Code != 0 {
		return fmt.Errorf("stapler validation failed")
	}
	logx.Success("Notarization and stapling verification passed")
	return nil
}

// MacOSSign is the sign_macos pipeline module.
type MacOSSign struct{}

func NewMacOSSign() *MacOSSign { return &MacOSSign{} }

func (MacOSSign) Name() string        { return "sign_macos" }
func (MacOSSign) Description() string { return "Sign and notarize macOS application" }

func (MacOSSign) Validate(ctx *buildctx.Context) error {
	if !ctx.Platform.IsMacOS() {
		return fmt.Errorf("macOS signing requires macOS")
	}
	appPath := ctx.AppPath()
	if _, err := os.Stat(appPath); err != nil {
		return fmt.Errorf("app not found at: %s", appPath)
	}
	if !CheckSigningEnvironment() {
		return fmt.Errorf("required signing environment variables not set")
	}
	return nil
}

func (MacOSSign) Execute(ctx *buildctx.Context) error {
	logx.Info(strings.Repeat("=", 70))
	logx.Info("🚀 Starting signing process for BrowserOS...")
	logx.Info(strings.Repeat("=", 70))

	UnlockKeychain(ctx)
	appPath := ctx.AppPath()

	if problems := VerifyServerResourcesBundle(appPath, ctx.ChromiumSrc); len(problems) > 0 {
		return fmt.Errorf(
			"app bundle does not match staged server resources (signing a stale build?):\n  %s",
			strings.Join(problems, "\n  "))
	}

	logx.Info("🧹 Clearing extended attributes...")
	if _, err := run(ctx, "xattr", "-cs", appPath); err != nil {
		return err
	}
	if err := SignAllComponents(ctx, appPath, envx.MacOSCertificateName()); err != nil {
		return err
	}
	if err := VerifySignature(ctx, appPath); err != nil {
		return err
	}
	if err := NotarizeApp(ctx, appPath); err != nil {
		return err
	}

	ctx.AddArtifact("signed_app", appPath)
	logx.Success("Application signed and notarized successfully")
	return nil
}
