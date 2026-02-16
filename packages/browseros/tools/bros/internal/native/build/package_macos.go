package build

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
)

func runPackageMacOS(ctx *Context) error {
	if runtime.GOOS != "darwin" {
		return fmt.Errorf("DMG creation requires macOS")
	}

	appPath := ctx.BrowserOSAppPath()
	if !dirExists(appPath) {
		return fmt.Errorf("app not found: %s", appPath)
	}

	if err := os.MkdirAll(ctx.DistDir(), 0o755); err != nil {
		return fmt.Errorf("creating dist dir: %w", err)
	}

	dmgName, err := ctx.ArtifactName("dmg")
	if err != nil {
		return err
	}
	dmgPath := filepath.Join(ctx.DistDir(), dmgName)
	pkgDmgPath := ctx.PkgDMGPath()

	if ctx.SignedApp {
		if err := validateMacOSSigningEnv(ctx); err != nil {
			return err
		}
		if err := createSignedNotarizedDMG(appPath, dmgPath, ctx.Env.MacOSCertificateName, pkgDmgPath, "notarytool-profile"); err != nil {
			return err
		}
		return nil
	}

	return createDMG(appPath, dmgPath, appBaseName, pkgDmgPath)
}

func createSignedNotarizedDMG(appPath, dmgPath, certificate, pkgDmgPath, keychainProfile string) error {
	if err := createDMG(appPath, dmgPath, appBaseName, pkgDmgPath); err != nil {
		return err
	}
	if err := signDMG(dmgPath, certificate); err != nil {
		return err
	}
	if err := notarizeDMG(dmgPath, keychainProfile); err != nil {
		return err
	}
	return nil
}

func createDMG(appPath, dmgPath, volumeName, pkgDmgPath string) error {
	if !dirExists(appPath) {
		return fmt.Errorf("app not found at %s", appPath)
	}

	if err := os.MkdirAll(filepath.Dir(dmgPath), 0o755); err != nil {
		return fmt.Errorf("creating dmg directory: %w", err)
	}
	if fileExists(dmgPath) {
		if err := os.Remove(dmgPath); err != nil {
			return fmt.Errorf("removing existing dmg: %w", err)
		}
	}

	cmd := []string{}
	useChromiumPkgDMG := fileExists(pkgDmgPath)
	if useChromiumPkgDMG {
		cmd = append(cmd, pkgDmgPath)
	} else {
		systemPkgDMG, err := exec.LookPath("pkg-dmg")
		if err != nil {
			return fmt.Errorf("no pkg-dmg tool found")
		}
		cmd = append(cmd, systemPkgDMG)
	}

	args := []string{
		"--sourcefile", "--source", appPath,
		"--target", dmgPath,
		"--volname", volumeName,
		"--symlink", "/Applications:/Applications",
		"--format", "UDBZ",
	}
	if useChromiumPkgDMG {
		args = append(args, "--verbosity", "2")
	}

	if err := runCmd("", cmd[0], args...); err != nil {
		return fmt.Errorf("creating dmg: %w", err)
	}
	return nil
}

func signDMG(dmgPath, certificate string) error {
	if !fileExists(dmgPath) {
		return fmt.Errorf("dmg not found at %s", dmgPath)
	}

	if err := runCmd("", "codesign", "--sign", certificate, "--force", "--timestamp", dmgPath); err != nil {
		return fmt.Errorf("signing dmg: %w", err)
	}

	verifyRes, err := runCmdCapture("", "codesign", "-vvv", dmgPath)
	if err != nil {
		return err
	}
	if verifyRes.ExitCode != 0 {
		return fmt.Errorf("dmg signature verification failed: %s", firstNonEmpty(verifyRes.Stderr, verifyRes.Stdout))
	}

	return nil
}

func notarizeDMG(dmgPath, keychainProfile string) error {
	if !fileExists(dmgPath) {
		return fmt.Errorf("dmg not found at %s", dmgPath)
	}

	submitRes, err := runCmdCapture("", "xcrun", "notarytool", "submit", dmgPath, "--keychain-profile", keychainProfile, "--wait")
	if err != nil {
		return err
	}
	if submitRes.ExitCode != 0 {
		return fmt.Errorf("dmg notarization submission failed: %s", firstNonEmpty(submitRes.Stderr, submitRes.Stdout))
	}
	if !strings.Contains(strings.ToLower(submitRes.Stdout), "status: accepted") {
		return fmt.Errorf("dmg notarization failed: %s", submitRes.Stdout)
	}

	stapleRes, err := runCmdCapture("", "xcrun", "stapler", "staple", dmgPath)
	if err != nil {
		return err
	}
	if stapleRes.ExitCode != 0 {
		return fmt.Errorf("failed to staple dmg: %s", firstNonEmpty(stapleRes.Stderr, stapleRes.Stdout))
	}

	validateRes, err := runCmdCapture("", "xcrun", "stapler", "validate", dmgPath)
	if err != nil {
		return err
	}
	if validateRes.ExitCode != 0 {
		return fmt.Errorf("dmg stapling verification failed: %s", firstNonEmpty(validateRes.Stderr, validateRes.Stdout))
	}

	spctlRes, err := runCmdCapture("", "spctl", "-a", "-vvv", "-t", "open", "--context", "context:primary-signature", dmgPath)
	if err != nil {
		return err
	}
	if spctlRes.ExitCode != 0 {
		return fmt.Errorf("dmg security assessment failed: %s", firstNonEmpty(spctlRes.Stderr, spctlRes.Stdout))
	}

	return nil
}
