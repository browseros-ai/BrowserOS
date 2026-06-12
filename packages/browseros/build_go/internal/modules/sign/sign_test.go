package sign

import (
	"crypto/ed25519"
	"encoding/base64"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/browseros-ai/BrowserOS/packages/browseros/build_go/internal/buildctx"
	"github.com/browseros-ai/BrowserOS/packages/browseros/build_go/internal/execx"
	"github.com/browseros-ai/BrowserOS/packages/browseros/build_go/internal/platform"
	"github.com/browseros-ai/BrowserOS/packages/browseros/build_go/internal/sparkle"
)

var (
	macArm = platform.Platform{OS: "macos", Arch: "arm64"}
	winX64 = platform.Platform{OS: "windows", Arch: "x64"}
)

func writeFile(t *testing.T, path, content string) {
	t.Helper()
	if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, []byte(content), 0o644); err != nil {
		t.Fatal(err)
	}
}

func writeExec(t *testing.T, path string) {
	t.Helper()
	writeFile(t, path, "#!/bin/sh\n")
	if err := os.Chmod(path, 0o755); err != nil {
		t.Fatal(err)
	}
}

func fixtureCtx(t *testing.T, plat platform.Platform) (*buildctx.Context, *execx.RecordingRunner) {
	t.Helper()
	root := t.TempDir()
	writeFile(t, filepath.Join(root, "pyproject.toml"), "name = \"browseros\"\n")
	writeFile(t, filepath.Join(root, "CHROMIUM_VERSION"), "MAJOR=148\nMINOR=0\nBUILD=7778\nPATCH=97\n")
	writeFile(t, filepath.Join(root, "build", "config", "BROWSEROS_BUILD_OFFSET"), "162\n")
	writeFile(t, filepath.Join(root, "resources", "BROWSEROS_VERSION"), "BROWSEROS_MAJOR=0\nBROWSEROS_MINOR=46\nBROWSEROS_BUILD=17\nBROWSEROS_PATCH=0\n")

	src := filepath.Join(t.TempDir(), "src")
	os.MkdirAll(src, 0o755)

	rec := &execx.RecordingRunner{}
	ctx, err := buildctx.New(buildctx.Options{
		ChromiumSrc: src, Architecture: plat.Arch, BuildType: "release",
		Platform: &plat, RootDir: root, Runner: rec,
	})
	if err != nil {
		t.Fatal(err)
	}
	return ctx, rec
}

// buildFixtureApp stages a minimal BrowserOS.app bundle with one of each
// component category.
func buildFixtureApp(t *testing.T, ctx *buildctx.Context) string {
	t.Helper()
	app := ctx.AppPath()
	fw := filepath.Join(app, "Contents", "Frameworks")
	browserFW := filepath.Join(fw, "BrowserOS Framework.framework")
	versioned := filepath.Join(browserFW, "Versions", ctx.BrowserOSChromiumVersion)

	writeExec(t, filepath.Join(app, "Contents", "MacOS", "BrowserOS"))
	writeExec(t, filepath.Join(versioned, "Helpers", "BrowserOS Helper (Renderer).app", "Contents", "MacOS", "helper"))
	writeExec(t, filepath.Join(versioned, "Helpers", "chrome_crashpad_handler"))
	writeFile(t, filepath.Join(versioned, "Libraries", "libEGL.dylib"), "dylib")
	writeFile(t, filepath.Join(fw, "Sparkle.framework", "Modules", "module"), "x")
	writeExec(t, filepath.Join(fw, "Sparkle.framework", "Versions", "B", "Autoupdate"))
	writeFile(t, filepath.Join(fw, "Sparkle.framework", "XPCServices", "Downloader.xpc", "Contents", "Info.plist"), "<plist/>")
	writeExec(t, filepath.Join(app, "Contents", "Resources", "BrowserOSServer", "default", "resources", "bin", "browseros_server"))
	writeExec(t, filepath.Join(app, "Contents", "Resources", "BrowserOSServer", "default", "resources", "bin", "codex"))
	return app
}

func TestFindComponentsToSignDiscoversAllCategories(t *testing.T) {
	ctx, _ := fixtureCtx(t, macArm)
	app := buildFixtureApp(t, ctx)

	c := FindComponentsToSign(ctx, app)
	if len(c.Helpers) != 1 || !strings.Contains(c.Helpers[0], "Helper (Renderer).app") {
		t.Errorf("helpers = %v", c.Helpers)
	}
	if len(c.XPCServices) != 1 || !strings.Contains(c.XPCServices[0], "Downloader.xpc") {
		t.Errorf("xpc = %v", c.XPCServices)
	}
	var execNames []string
	for _, exe := range c.Executables {
		execNames = append(execNames, filepath.Base(exe))
	}
	joined := strings.Join(execNames, ",")
	for _, want := range []string{"chrome_crashpad_handler", "Autoupdate", "browseros_server", "codex"} {
		if !strings.Contains(joined, want) {
			t.Errorf("executables missing %s: %v", want, execNames)
		}
	}
	if len(c.Dylibs) != 1 || !strings.Contains(c.Dylibs[0], "libEGL.dylib") {
		t.Errorf("dylibs = %v", c.Dylibs)
	}
	if len(c.Frameworks) < 2 {
		t.Errorf("frameworks = %v", c.Frameworks)
	}
}

func TestIdentifierForComponent(t *testing.T) {
	cases := map[string]string{
		"/x/Sparkle.framework/Versions/B/Autoupdate":      "org.sparkle-project.Autoupdate",
		"/x/Helpers/chrome_crashpad_handler":              "com.browseros.crashpad_handler",
		"/x/Helpers/BrowserOS Helper (Renderer).app":      "com.browseros.helper.renderer",
		"/x/Helpers/BrowserOS Helper (GPU).app":           "com.browseros.helper.gpu",
		"/x/Frameworks/BrowserOS Framework.framework":     "com.browseros.framework",
		"/x/Libraries/libEGL.dylib":                       "com.browseros.libEGL",
		"/x/BrowserOSServer/default/resources/bin/codex":  "com.browseros.codex",
		"/x/BrowserOSServer/default/resources/bin/claude": "com.browseros.claude",
	}
	for path, want := range cases {
		if got := IdentifierForComponent(path); got != want {
			t.Errorf("IdentifierForComponent(%s) = %q, want %q", path, got, want)
		}
	}
}

func TestSigningOptions(t *testing.T) {
	cases := map[string]string{
		"/x/Sparkle.framework/XPCServices/Downloader.xpc": "runtime",
		"/x/Helpers/BrowserOS Helper (Renderer).app":      "restrict,kill,runtime",
		"/x/Libraries/libEGL.dylib":                       "restrict,library,runtime,kill",
		"/x/bin/browseros_server":                         "runtime",
		"/x/Contents/MacOS/SomethingElse":                 "runtime",
	}
	for path, want := range cases {
		if got := SigningOptions(path); got != want {
			t.Errorf("SigningOptions(%s) = %q, want %q", path, got, want)
		}
	}
}

func TestSignAllComponentsOrderAndArgs(t *testing.T) {
	ctx, rec := fixtureCtx(t, macArm)
	app := buildFixtureApp(t, ctx)
	writeFile(t, filepath.Join(ctx.EntitlementsDir(), "browseros-executable-entitlements.plist"), "<plist/>")
	writeFile(t, filepath.Join(ctx.EntitlementsDir(), "helper-renderer-entitlements.plist"), "<plist/>")
	writeFile(t, filepath.Join(ctx.EntitlementsDir(), "app-entitlements.plist"), "<plist/>")

	if err := SignAllComponents(ctx, app, "Developer ID Application: Test"); err != nil {
		t.Fatal(err)
	}

	// Drop the read-only Mach-O probes signComponent uses for routing; only
	// the mutating codesign sequence is under test here. (No Handler is set,
	// so probes get zero-value results: machoArchs sees no archs and the
	// per-slice path — lipo -thin/-create — never fires.)
	var argv []string
	for _, cmd := range rec.Argv() {
		if strings.HasPrefix(cmd, "lipo -archs ") || strings.HasPrefix(cmd, "otool ") {
			continue
		}
		argv = append(argv, cmd)
	}
	for _, cmd := range argv {
		if !strings.HasPrefix(cmd, "codesign --sign Developer ID Application: Test --force --timestamp") {
			t.Errorf("unexpected command: %q", cmd)
		}
	}

	find := func(substr string) int {
		for i, cmd := range argv {
			if strings.Contains(cmd, substr) {
				return i
			}
		}
		t.Fatalf("no command containing %q in:\n%s", substr, strings.Join(argv, "\n"))
		return -1
	}

	// Bottom-up order: XPC before frameworks; Sparkle.framework before
	// BrowserOS Framework; main exe before the final app bundle.
	xpcIdx := find("Downloader.xpc")
	sparkleFwIdx := find("--identifier org.sparkle-project.Sparkle ")
	mainFwIdx := find("--identifier com.browseros.framework ")
	mainExeIdx := find("Contents/MacOS/BrowserOS")
	bundleIdx := find("--requirements")
	if !(xpcIdx < sparkleFwIdx && sparkleFwIdx < mainFwIdx && mainFwIdx < mainExeIdx && mainExeIdx < bundleIdx) {
		t.Errorf("sign order wrong: xpc=%d sparkleFw=%d mainFw=%d mainExe=%d bundle=%d\n%s",
			xpcIdx, sparkleFwIdx, mainFwIdx, mainExeIdx, bundleIdx, strings.Join(argv, "\n"))
	}

	// Server binary gets its entitlements; renderer helper gets its plist.
	serverIdx := find("bin/browseros_server")
	if !strings.Contains(argv[serverIdx], "browseros-executable-entitlements.plist") {
		t.Errorf("server binary should be signed with entitlements: %q", argv[serverIdx])
	}
	helperIdx := find("Helper (Renderer).app")
	if !strings.Contains(argv[helperIdx], "helper-renderer-entitlements.plist") {
		t.Errorf("renderer helper should use renderer entitlements: %q", argv[helperIdx])
	}
	// Final bundle uses the hardened options + requirements + app entitlements.
	if !strings.Contains(argv[bundleIdx], "--options restrict,library,runtime,kill") ||
		!strings.Contains(argv[bundleIdx], "app-entitlements.plist") {
		t.Errorf("bundle sign command: %q", argv[bundleIdx])
	}
}

func TestNotarizeAppSequence(t *testing.T) {
	ctx, rec := fixtureCtx(t, macArm)
	app := buildFixtureApp(t, ctx)
	t.Setenv("PROD_MACOS_NOTARIZATION_APPLE_ID", "dev@browseros.com")
	t.Setenv("PROD_MACOS_NOTARIZATION_TEAM_ID", "TEAM123")
	t.Setenv("PROD_MACOS_NOTARIZATION_PWD", "secret")

	rec.Handler = func(c execx.Cmd) (execx.Result, error) {
		if strings.Contains(c.String(), "notarytool submit") {
			return execx.Result{Stdout: "id: abc-123\nstatus: Accepted\n"}, nil
		}
		return execx.Result{}, nil
	}

	if err := NotarizeApp(ctx, app); err != nil {
		t.Fatal(err)
	}

	argv := rec.Argv()
	wantPrefixes := []string{
		"ditto -c -k --keepParent",
		"xcrun notarytool store-credentials notarytool-profile",
		"xcrun notarytool submit",
		"xcrun stapler staple",
		"spctl -a -vvv",
		"xcrun stapler validate",
	}
	if len(argv) != len(wantPrefixes) {
		t.Fatalf("got %d commands:\n%s", len(argv), strings.Join(argv, "\n"))
	}
	for i, prefix := range wantPrefixes {
		if !strings.HasPrefix(argv[i], prefix) {
			t.Errorf("cmd[%d] = %q, want prefix %q", i, argv[i], prefix)
		}
	}
	if !strings.Contains(argv[2], "--keychain-profile notarytool-profile --wait") {
		t.Errorf("submit should use keychain profile: %q", argv[2])
	}
}

func TestNotarizeAppFailsWhenNotAccepted(t *testing.T) {
	ctx, rec := fixtureCtx(t, macArm)
	app := buildFixtureApp(t, ctx)
	rec.Handler = func(c execx.Cmd) (execx.Result, error) {
		if strings.Contains(c.String(), "notarytool submit") {
			return execx.Result{Stdout: "id: xyz\nstatus: Invalid\n"}, nil
		}
		return execx.Result{}, nil
	}
	err := NotarizeApp(ctx, app)
	if err == nil || !strings.Contains(err.Error(), "Accepted") {
		t.Errorf("err = %v", err)
	}
}

func TestVerifyServerResourcesBundleDetectsDrift(t *testing.T) {
	ctx, _ := fixtureCtx(t, macArm)
	app := ctx.AppPath()

	// Staged tree has two files (one executable); bundle is missing one and
	// lost the exec bit on the other.
	staged := filepath.Join(ctx.ChromiumSrc, "chrome", "browser", "browseros", "server", "resources")
	writeExec(t, filepath.Join(staged, "bin", "browseros_server"))
	writeFile(t, filepath.Join(staged, "config.json"), "{}")
	writeFile(t, filepath.Join(app, "Contents", "Resources", "BrowserOSServer", "default", "resources", "bin", "browseros_server"), "x")

	problems := VerifyServerResourcesBundle(app, ctx.ChromiumSrc)
	joined := strings.Join(problems, "\n")
	if !strings.Contains(joined, "lost executable bit in app bundle: bin/browseros_server") {
		t.Errorf("missing exec-bit problem: %v", problems)
	}
	if !strings.Contains(joined, "missing from app bundle: config.json") {
		t.Errorf("missing file problem: %v", problems)
	}

	// No staged tree → no problems (sign-only flows).
	os.RemoveAll(staged)
	if problems := VerifyServerResourcesBundle(app, ctx.ChromiumSrc); len(problems) != 0 {
		t.Errorf("problems without staged tree = %v", problems)
	}
}

func TestSparkleSignRoundTrip(t *testing.T) {
	// Generate a key, sign a fixture DMG via the module, verify with the
	// derived public key.
	seed := make([]byte, ed25519.SeedSize)
	for i := range seed {
		seed[i] = byte(i + 1)
	}
	key := ed25519.NewKeyFromSeed(seed)
	t.Setenv("SPARKLE_PRIVATE_KEY", base64.StdEncoding.EncodeToString(seed))

	ctx, _ := fixtureCtx(t, macArm)
	dmg := filepath.Join(ctx.DistDir(), "BrowserOS_v0.46.17_arm64.dmg")
	writeFile(t, dmg, "dmg-bytes-payload")

	module := SparkleSign{}
	if err := module.Validate(ctx); err != nil {
		t.Fatal(err)
	}
	if err := module.Execute(ctx); err != nil {
		t.Fatal(err)
	}

	sig, ok := ctx.SparkleSignatures["BrowserOS_v0.46.17_arm64.dmg"]
	if !ok {
		t.Fatalf("signature not recorded: %v", ctx.SparkleSignatures)
	}
	if sig.Length != int64(len("dmg-bytes-payload")) {
		t.Errorf("length = %d", sig.Length)
	}
	sigBytes, err := base64.StdEncoding.DecodeString(sig.Signature)
	if err != nil {
		t.Fatal(err)
	}
	if !ed25519.Verify(key.Public().(ed25519.PublicKey), []byte("dmg-bytes-payload"), sigBytes) {
		t.Error("signature does not verify with the public key")
	}
}

func TestSparkleParsePrivateKeyFormats(t *testing.T) {
	seed := make([]byte, 32)
	for i := range seed {
		seed[i] = byte(i)
	}
	want := ed25519.NewKeyFromSeed(seed)

	// base64(32-byte seed)
	key, err := sparkle.ParsePrivateKey(base64.StdEncoding.EncodeToString(seed))
	if err != nil || !key.Equal(want) {
		t.Errorf("32-byte b64: %v", err)
	}
	// base64(64-byte seed+pub)
	full := append(append([]byte{}, seed...), want.Public().(ed25519.PublicKey)...)
	key, err = sparkle.ParsePrivateKey(base64.StdEncoding.EncodeToString(full))
	if err != nil || !key.Equal(want) {
		t.Errorf("64-byte b64: %v", err)
	}
	// Wrong length errors.
	if _, err := sparkle.ParsePrivateKey(base64.StdEncoding.EncodeToString([]byte("short"))); err == nil {
		t.Error("short key should error")
	}
}

func TestWindowsSignValidateAndServerPaths(t *testing.T) {
	ctx, _ := fixtureCtx(t, winX64)
	for _, name := range []string{"CODE_SIGN_TOOL_PATH", "ESIGNER_USERNAME", "ESIGNER_PASSWORD", "ESIGNER_TOTP_SECRET"} {
		t.Setenv(name, "")
		os.Unsetenv(name)
	}
	os.MkdirAll(ctx.OutDirAbs(), 0o755)

	if err := (WindowsSign{}).Validate(ctx); err == nil || !strings.Contains(err.Error(), "CODE_SIGN_TOOL_PATH") {
		t.Errorf("err = %v", err)
	}
	t.Setenv("CODE_SIGN_TOOL_PATH", t.TempDir())
	err := (WindowsSign{}).Validate(ctx)
	if err == nil || !strings.Contains(err.Error(), "ESIGNER_USERNAME") {
		t.Errorf("err = %v", err)
	}

	paths := ServerBinaryPaths(ctx.OutDirAbs())
	if len(paths) != 3 || !strings.HasSuffix(paths[0], "browseros_server.exe") {
		t.Errorf("server paths = %v", paths)
	}
	mac := MacOSSign{}
	if err := mac.Validate(ctx); err == nil || !strings.Contains(err.Error(), "requires macOS") {
		t.Errorf("macos sign on windows: %v", err)
	}
}

func TestLinuxSignIsNoOp(t *testing.T) {
	ctx, rec := fixtureCtx(t, platform.Platform{OS: "linux", Arch: "x64"})
	if err := (LinuxSign{}).Validate(ctx); err != nil {
		t.Fatal(err)
	}
	if err := (LinuxSign{}).Execute(ctx); err != nil {
		t.Fatal(err)
	}
	if len(rec.Cmds) != 0 {
		t.Errorf("linux sign should run nothing: %v", rec.Argv())
	}
}

// --- per-slice signing of asymmetric fat Mach-Os ---

func hasArg(args []string, want string) bool {
	for _, a := range args {
		if a == want {
			return true
		}
	}
	return false
}

func argAfter(args []string, flag string) string {
	for i, a := range args {
		if a == flag && i+1 < len(args) {
			return args[i+1]
		}
	}
	return ""
}

// sliceHandler fakes lipo/otool/codesign: two archs, an embedded Info.plist
// only on the archs in plistArchs, and lipo -output materialized on disk.
func sliceHandler(t *testing.T, plistArchs map[string]bool, codesignCode int, machO bool) func(execx.Cmd) (execx.Result, error) {
	t.Helper()
	return func(c execx.Cmd) (execx.Result, error) {
		args := c.Args
		switch {
		case args[0] == "lipo" && args[1] == "-archs":
			if !machO {
				return execx.Result{Code: 1}, nil
			}
			return execx.Result{Stdout: "x86_64 arm64\n"}, nil
		case args[0] == "otool":
			if plistArchs[args[2]] {
				return execx.Result{Stdout: "sectname __info_plist\n"}, nil
			}
			return execx.Result{Stdout: "sectname __text\n"}, nil
		case args[0] == "lipo":
			payload := "thin"
			if args[1] == "-create" {
				payload = "signed-fat"
			}
			if out := argAfter(args, "-output"); out != "" {
				if err := os.WriteFile(out, []byte(payload), 0o644); err != nil {
					t.Fatal(err)
				}
			}
			return execx.Result{}, nil
		case args[0] == "codesign":
			return execx.Result{Code: codesignCode}, nil
		}
		return execx.Result{}, nil
	}
}

func TestSignComponentAsymmetricFatSignsPerSlice(t *testing.T) {
	ctx, rec := fixtureCtx(t, macArm)
	dir := t.TempDir()
	component := filepath.Join(dir, "claude")
	writeFile(t, component, "original-fat")
	if err := os.Chmod(component, 0o755); err != nil {
		t.Fatal(err)
	}
	rec.Handler = sliceHandler(t, map[string]bool{"arm64": true}, 0, true)

	if err := signComponent(ctx, component, "Cert", "com.browseros.claude", "runtime", ""); err != nil {
		t.Fatal(err)
	}

	var codesigns, thins, creates [][]string
	for _, c := range rec.Cmds {
		switch {
		case c.Args[0] == "codesign":
			codesigns = append(codesigns, c.Args)
		case c.Args[0] == "lipo" && hasArg(c.Args, "-thin"):
			thins = append(thins, c.Args)
		case c.Args[0] == "lipo" && c.Args[1] == "-create":
			creates = append(creates, c.Args)
		}
	}
	if len(codesigns) != 2 {
		t.Fatalf("want 2 codesign calls (one per slice), got %d: %v", len(codesigns), rec.Argv())
	}
	for _, args := range codesigns {
		if args[len(args)-1] == component {
			t.Errorf("codesign ran on the fat file instead of a thin slice: %v", args)
		}
		for _, want := range []string{"--force", "--timestamp", "--identifier", "com.browseros.claude", "--options", "runtime"} {
			if !hasArg(args, want) {
				t.Errorf("codesign missing %q: %v", want, args)
			}
		}
	}
	thinArchs := map[string]bool{}
	for _, args := range thins {
		thinArchs[argAfter(args, "-thin")] = true
	}
	if !thinArchs["x86_64"] || !thinArchs["arm64"] {
		t.Errorf("want thin extraction for both archs, got %v", thinArchs)
	}
	if len(creates) != 1 {
		t.Fatalf("want 1 lipo -create, got %d", len(creates))
	}
	data, err := os.ReadFile(component)
	if err != nil {
		t.Fatal(err)
	}
	if string(data) != "signed-fat" {
		t.Errorf("component not replaced with reassembled fat: %q", data)
	}
	st, err := os.Stat(component)
	if err != nil {
		t.Fatal(err)
	}
	if st.Mode().Perm() != 0o755 {
		t.Errorf("mode not preserved: %v", st.Mode().Perm())
	}
	entries, err := os.ReadDir(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(entries) != 1 {
		t.Errorf("temp artifacts left behind: %v", entries)
	}
}

func TestSignComponentSymmetricFatSingleCodesign(t *testing.T) {
	for name, plistArchs := range map[string]map[string]bool{
		"both_have_plist": {"x86_64": true, "arm64": true},
		"neither_has":     {},
	} {
		t.Run(name, func(t *testing.T) {
			ctx, rec := fixtureCtx(t, macArm)
			dir := t.TempDir()
			component := filepath.Join(dir, "claude")
			writeFile(t, component, "original-fat")
			rec.Handler = sliceHandler(t, plistArchs, 0, true)

			if err := signComponent(ctx, component, "Cert", "", "", ""); err != nil {
				t.Fatal(err)
			}
			var codesigns [][]string
			for _, c := range rec.Cmds {
				if c.Args[0] == "codesign" {
					codesigns = append(codesigns, c.Args)
				}
				if c.Args[0] == "lipo" && (hasArg(c.Args, "-thin") || c.Args[1] == "-create") {
					t.Errorf("symmetric fat must not be split: %v", c.Args)
				}
			}
			if len(codesigns) != 1 || codesigns[0][len(codesigns[0])-1] != component {
				t.Errorf("want a single codesign on the fat file, got %v", codesigns)
			}
			data, _ := os.ReadFile(component)
			if string(data) != "original-fat" {
				t.Errorf("file must not be rewritten: %q", data)
			}
		})
	}
}

func TestSignComponentThinSingleArchSingleCodesign(t *testing.T) {
	ctx, rec := fixtureCtx(t, macArm)
	dir := t.TempDir()
	component := filepath.Join(dir, "claude")
	writeFile(t, component, "thin-binary")
	rec.Handler = func(c execx.Cmd) (execx.Result, error) {
		if c.Args[0] == "lipo" && c.Args[1] == "-archs" {
			return execx.Result{Stdout: "arm64\n"}, nil
		}
		return execx.Result{}, nil
	}

	if err := signComponent(ctx, component, "Cert", "", "", ""); err != nil {
		t.Fatal(err)
	}
	var codesigns [][]string
	for _, c := range rec.Cmds {
		if c.Args[0] == "codesign" {
			codesigns = append(codesigns, c.Args)
		}
	}
	if len(codesigns) != 1 || codesigns[0][len(codesigns[0])-1] != component {
		t.Errorf("want a single codesign on the thin file, got %v", codesigns)
	}
	data, _ := os.ReadFile(component)
	if string(data) != "thin-binary" {
		t.Errorf("file must not be rewritten: %q", data)
	}
}

func TestSignComponentNonMachOSingleCodesign(t *testing.T) {
	ctx, rec := fixtureCtx(t, macArm)
	dir := t.TempDir()
	component := filepath.Join(dir, "wrapper")
	writeExec(t, component)
	rec.Handler = sliceHandler(t, nil, 0, false)

	if err := signComponent(ctx, component, "Cert", "", "", ""); err != nil {
		t.Fatal(err)
	}
	var codesigns [][]string
	for _, c := range rec.Cmds {
		if c.Args[0] == "codesign" {
			codesigns = append(codesigns, c.Args)
		}
	}
	if len(codesigns) != 1 || codesigns[0][len(codesigns[0])-1] != component {
		t.Errorf("want a single codesign on the file, got %v", codesigns)
	}
}

func TestSignComponentPerSliceCodesignFailureKeepsOriginal(t *testing.T) {
	ctx, rec := fixtureCtx(t, macArm)
	dir := t.TempDir()
	component := filepath.Join(dir, "claude")
	writeFile(t, component, "original-fat")
	rec.Handler = sliceHandler(t, map[string]bool{"arm64": true}, 1, true)

	if err := signComponent(ctx, component, "Cert", "", "", ""); err == nil {
		t.Fatal("want error when a slice fails to sign")
	}
	data, _ := os.ReadFile(component)
	if string(data) != "original-fat" {
		t.Errorf("original file must be left in place: %q", data)
	}
	entries, _ := os.ReadDir(dir)
	if len(entries) != 1 {
		t.Errorf("temp artifacts left behind: %v", entries)
	}
}

func TestVerifySignatureFailsOnInvalidComponent(t *testing.T) {
	ctx, rec := fixtureCtx(t, macArm)
	appPath := filepath.Join(t.TempDir(), "BrowserOS.app")
	claude := filepath.Join(appPath, "Contents", "Resources", "BrowserOSServer",
		"default", "resources", "bin", "third_party", "claude")
	writeExec(t, claude)

	rec.Handler = func(c execx.Cmd) (execx.Result, error) {
		if c.Args[0] == "codesign" && c.Args[len(c.Args)-1] == claude {
			return execx.Result{Code: 1}, nil
		}
		return execx.Result{}, nil
	}
	err := VerifySignature(ctx, appPath)
	if err == nil || !strings.Contains(err.Error(), "claude") {
		t.Fatalf("want component verification failure naming claude, got %v", err)
	}
}

func TestVerifySignaturePassesAndChecksEachComponent(t *testing.T) {
	ctx, rec := fixtureCtx(t, macArm)
	appPath := filepath.Join(t.TempDir(), "BrowserOS.app")
	claude := filepath.Join(appPath, "Contents", "Resources", "BrowserOSServer",
		"default", "resources", "bin", "third_party", "claude")
	writeExec(t, claude)

	if err := VerifySignature(ctx, appPath); err != nil {
		t.Fatal(err)
	}
	found := false
	for _, c := range rec.Cmds {
		if c.Args[0] == "codesign" && hasArg(c.Args, "--verify") && c.Args[len(c.Args)-1] == claude {
			found = true
		}
	}
	if !found {
		t.Errorf("claude was not verified: %v", rec.Argv())
	}
}
