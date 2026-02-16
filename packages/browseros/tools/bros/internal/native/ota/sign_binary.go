package ota

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"

	"bros/internal/native/common"
)

func signBinary(binaryPath string, platform ServerPlatform, env common.EnvConfig, repoRoot string) error {
	switch platform.OS {
	case "macos":
		return signMacOSBinary(binaryPath, env, repoRoot, platform)
	case "windows":
		return signWindowsBinary(binaryPath, env)
	case "linux":
		fmt.Printf("No code signing for Linux binary: %s\n", filepath.Base(binaryPath))
		return nil
	default:
		return nil
	}
}

func signMacOSBinary(binaryPath string, env common.EnvConfig, repoRoot string, platform ServerPlatform) error {
	if runtime.GOOS != "darwin" {
		fmt.Printf("macOS signing requires macOS - skipping %s\n", platform.Name)
		return nil
	}

	if env.MacOSCertificateName == "" {
		return fmt.Errorf("MACOS_CERTIFICATE_NAME required for signing on macOS")
	}

	args := []string{
		"--sign", env.MacOSCertificateName,
		"--force",
		"--timestamp",
		"--identifier", "com.browseros." + strings.TrimSuffix(filepath.Base(binaryPath), filepath.Ext(binaryPath)),
		"--options", "runtime",
	}

	if entitlements := findEntitlementsPath(repoRoot); entitlements != "" {
		args = append(args, "--entitlements", entitlements)
	}
	args = append(args, binaryPath)

	if output, err := runCommandCapture("", "codesign", args...); err != nil {
		return fmt.Errorf("codesign failed for %s: %s", filepath.Base(binaryPath), strings.TrimSpace(output))
	}

	if output, err := runCommandCapture("", "codesign", "--verify", "--verbose=2", binaryPath); err != nil {
		return fmt.Errorf("codesign verification failed for %s: %s", filepath.Base(binaryPath), strings.TrimSpace(output))
	}

	if err := notarizeMacOSBinary(binaryPath, env); err != nil {
		return err
	}

	return nil
}

func notarizeMacOSBinary(binaryPath string, env common.EnvConfig) error {
	if runtime.GOOS != "darwin" {
		return nil
	}

	if env.MacOSNotarizationAppleID == "" || env.MacOSNotarizationTeamID == "" || env.MacOSNotarizationPassword == "" {
		return fmt.Errorf("missing notarization credentials: PROD_MACOS_NOTARIZATION_APPLE_ID, PROD_MACOS_NOTARIZATION_TEAM_ID, PROD_MACOS_NOTARIZATION_PWD")
	}

	tempZip, err := os.CreateTemp("", "browseros-notary-*.zip")
	if err != nil {
		return fmt.Errorf("creating temporary zip for notarization: %w", err)
	}
	tempZipPath := tempZip.Name()
	_ = tempZip.Close()
	defer os.Remove(tempZipPath)

	if output, err := runCommandCapture("", "ditto", "-c", "-k", "--keepParent", binaryPath, tempZipPath); err != nil {
		return fmt.Errorf("failed to create notarization zip: %s", strings.TrimSpace(output))
	}

	_, _ = runCommandCapture("", "xcrun", "notarytool", "store-credentials", "notarytool-profile",
		"--apple-id", env.MacOSNotarizationAppleID,
		"--team-id", env.MacOSNotarizationTeamID,
		"--password", env.MacOSNotarizationPassword,
	)

	output, err := runCommandCapture("", "xcrun", "notarytool", "submit", tempZipPath,
		"--keychain-profile", "notarytool-profile",
		"--wait",
	)
	if err != nil {
		return fmt.Errorf("notarization failed: %s", strings.TrimSpace(output))
	}
	if !strings.Contains(output, "status: Accepted") {
		return fmt.Errorf("notarization was not accepted: %s", strings.TrimSpace(output))
	}

	return nil
}

func signWindowsBinary(binaryPath string, env common.EnvConfig) error {
	toolPath := ""
	switch {
	case env.CodeSignToolExe != "":
		toolPath = env.CodeSignToolExe
	case env.CodeSignToolPath != "":
		toolPath = filepath.Join(env.CodeSignToolPath, "CodeSignTool.bat")
	default:
		fmt.Printf("CODE_SIGN_TOOL_EXE not set - skipping Windows signing for %s\n", filepath.Base(binaryPath))
		return nil
	}

	if _, err := os.Stat(toolPath); err != nil {
		return fmt.Errorf("CodeSignTool not found at %s", toolPath)
	}

	if env.ESignerUsername == "" || env.ESignerPassword == "" || env.ESignerTOTPSecret == "" {
		return fmt.Errorf("missing eSigner credentials: ESIGNER_USERNAME, ESIGNER_PASSWORD, ESIGNER_TOTP_SECRET")
	}

	tempOutputDir := filepath.Join(filepath.Dir(binaryPath), "signed_temp")
	if err := os.MkdirAll(tempOutputDir, 0o755); err != nil {
		return fmt.Errorf("creating temporary signing directory: %w", err)
	}
	defer os.RemoveAll(tempOutputDir)

	args := []string{
		"sign",
		"-username", env.ESignerUsername,
		"-password", env.ESignerPassword,
	}
	if env.ESignerCredentialID != "" {
		args = append(args, "-credential_id", env.ESignerCredentialID)
	}
	args = append(args,
		"-totp_secret", env.ESignerTOTPSecret,
		"-input_file_path", binaryPath,
		"-output_dir_path", tempOutputDir,
		"-override",
	)

	output, err := runToolCapture(toolPath, filepath.Dir(toolPath), args...)
	if err != nil {
		return fmt.Errorf("CodeSignTool failed: %s", strings.TrimSpace(output))
	}
	if strings.Contains(output, "Error:") {
		return fmt.Errorf("CodeSignTool reported error: %s", strings.TrimSpace(output))
	}

	signedPath := filepath.Join(tempOutputDir, filepath.Base(binaryPath))
	if _, err := os.Stat(signedPath); err == nil {
		if err := os.Rename(signedPath, binaryPath); err != nil {
			if copyErr := copyFile(signedPath, binaryPath); copyErr != nil {
				return fmt.Errorf("moving signed binary into place: %w", err)
			}
		}
	}

	if runtime.GOOS == "windows" {
		pathForPS := strings.ReplaceAll(binaryPath, "'", "''")
		verifyOutput, verifyErr := runCommandCapture("", "powershell", "-Command", "(Get-AuthenticodeSignature '"+pathForPS+"').Status")
		if verifyErr != nil {
			return fmt.Errorf("signature verification failed: %s", strings.TrimSpace(verifyOutput))
		}
		if !strings.Contains(strings.TrimSpace(verifyOutput), "Valid") {
			return fmt.Errorf("signature verification failed: %s", strings.TrimSpace(verifyOutput))
		}
	}

	return nil
}

func findEntitlementsPath(repoRoot string) string {
	candidates := []string{
		filepath.Join(repoRoot, "resources", "entitlements", "browseros-executable-entitlements.plist"),
		filepath.Join(repoRoot, "packages", "browseros", "resources", "entitlements", "browseros-executable-entitlements.plist"),
	}

	for _, candidate := range candidates {
		if _, err := os.Stat(candidate); err == nil {
			return candidate
		}
	}

	return ""
}

func runToolCapture(toolPath string, workingDir string, args ...string) (string, error) {
	if runtime.GOOS == "windows" && strings.EqualFold(filepath.Ext(toolPath), ".bat") {
		cmdArgs := append([]string{"/C", toolPath}, args...)
		return runCommandCapture(workingDir, "cmd", cmdArgs...)
	}

	return runCommandCapture(workingDir, toolPath, args...)
}

func runCommandCapture(workingDir string, name string, args ...string) (string, error) {
	cmd := exec.Command(name, args...)
	if workingDir != "" {
		cmd.Dir = workingDir
	}

	var stdout bytes.Buffer
	var stderr bytes.Buffer
	cmd.Stdout = &stdout
	cmd.Stderr = &stderr

	err := cmd.Run()
	output := strings.TrimSpace(stdout.String())
	if serr := strings.TrimSpace(stderr.String()); serr != "" {
		if output == "" {
			output = serr
		} else {
			output = output + "\n" + serr
		}
	}

	if err != nil {
		return output, err
	}
	return output, nil
}

func copyFile(srcPath string, dstPath string) error {
	src, err := os.Open(srcPath)
	if err != nil {
		return err
	}
	defer src.Close()

	info, err := src.Stat()
	if err != nil {
		return err
	}

	dst, err := os.OpenFile(dstPath, os.O_CREATE|os.O_TRUNC|os.O_WRONLY, info.Mode())
	if err != nil {
		return err
	}
	defer dst.Close()

	if _, err := dst.ReadFrom(src); err != nil {
		return err
	}

	return nil
}
