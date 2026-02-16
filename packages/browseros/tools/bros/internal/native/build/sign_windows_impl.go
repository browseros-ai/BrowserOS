package build

import (
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strings"
)

var browserOSServerWindowsBinaries = []string{
	"browseros_server.exe",
	"codex.exe",
}

func runSignWindows(ctx *Context) error {
	if runtime.GOOS != "windows" {
		return fmt.Errorf("Windows signing requires Windows")
	}

	buildOutputDir := filepath.Join(ctx.ChromiumSrc, ctx.OutDir)
	if !dirExists(buildOutputDir) {
		return fmt.Errorf("build output directory not found: %s", buildOutputDir)
	}

	if strings.TrimSpace(ctx.Env.CodeSignToolPath) == "" && strings.TrimSpace(ctx.Env.CodeSignToolExe) == "" {
		return fmt.Errorf("CODE_SIGN_TOOL_PATH or CODE_SIGN_TOOL_EXE environment variable not set")
	}

	missing := make([]string, 0)
	if strings.TrimSpace(ctx.Env.ESignerUsername) == "" {
		missing = append(missing, "ESIGNER_USERNAME")
	}
	if strings.TrimSpace(ctx.Env.ESignerPassword) == "" {
		missing = append(missing, "ESIGNER_PASSWORD")
	}
	if strings.TrimSpace(ctx.Env.ESignerTOTPSecret) == "" {
		missing = append(missing, "ESIGNER_TOTP_SECRET")
	}
	if len(missing) > 0 {
		return fmt.Errorf("missing environment variables: %s", strings.Join(missing, ", "))
	}

	binaries := []string{filepath.Join(buildOutputDir, "chrome.exe")}
	for _, name := range browserOSServerWindowsBinaries {
		binaries = append(binaries, filepath.Join(buildOutputDir, "BrowserOSServer", "default", "resources", "bin", name))
	}

	existing := make([]string, 0, len(binaries))
	for _, binary := range binaries {
		if fileExists(binary) {
			existing = append(existing, binary)
		}
	}
	if len(existing) == 0 {
		return fmt.Errorf("no binaries found to sign")
	}

	if err := signWithCodeSignTool(existing, ctx); err != nil {
		return err
	}

	if err := buildMiniInstallerWindows(ctx); err != nil {
		return err
	}

	miniInstaller := filepath.Join(buildOutputDir, "mini_installer.exe")
	if !fileExists(miniInstaller) {
		return fmt.Errorf("mini_installer.exe not found: %s", miniInstaller)
	}

	if err := signWithCodeSignTool([]string{miniInstaller}, ctx); err != nil {
		return err
	}

	ctx.SignedApp = true
	return nil
}

func buildMiniInstallerWindows(ctx *Context) error {
	if err := runCmd(ctx.ChromiumSrc, "autoninja.bat", "-C", ctx.OutDir, "setup", "mini_installer"); err != nil {
		return fmt.Errorf("building setup/mini_installer: %w", err)
	}

	buildOutputDir := filepath.Join(ctx.ChromiumSrc, ctx.OutDir)
	if !fileExists(filepath.Join(buildOutputDir, "setup.exe")) || !fileExists(filepath.Join(buildOutputDir, "mini_installer.exe")) {
		return fmt.Errorf("build completed but setup.exe or mini_installer.exe is missing")
	}

	return nil
}

func signWithCodeSignTool(binaries []string, ctx *Context) error {
	toolPath, err := resolveCodeSignToolPath(ctx)
	if err != nil {
		return err
	}

	for _, binary := range binaries {
		tempOutputDir := filepath.Join(filepath.Dir(binary), "signed_temp")
		if err := os.MkdirAll(tempOutputDir, 0o755); err != nil {
			return fmt.Errorf("creating temp output dir for %s: %w", binary, err)
		}

		args := []string{
			"sign",
			"-username", ctx.Env.ESignerUsername,
			"-password", ctx.Env.ESignerPassword,
		}
		if strings.TrimSpace(ctx.Env.ESignerCredentialID) != "" {
			args = append(args, "-credential_id", ctx.Env.ESignerCredentialID)
		}
		args = append(args,
			"-totp_secret", ctx.Env.ESignerTOTPSecret,
			"-input_file_path", binary,
			"-output_dir_path", tempOutputDir,
			"-override",
		)

		result, err := runCodeSignCommand(toolPath, filepath.Dir(toolPath), args...)
		if err != nil {
			return err
		}
		if result.ExitCode != 0 {
			return fmt.Errorf("codesign tool failed for %s: %s", binary, firstNonEmpty(result.Stderr, result.Stdout))
		}
		if strings.Contains(result.Stdout, "Error:") {
			return fmt.Errorf("codesign tool reported an error for %s: %s", binary, strings.TrimSpace(result.Stdout))
		}

		signedFile := filepath.Join(tempOutputDir, filepath.Base(binary))
		if fileExists(signedFile) {
			if err := os.Rename(signedFile, binary); err != nil {
				return fmt.Errorf("moving signed file for %s: %w", binary, err)
			}
		}
		_ = os.RemoveAll(tempOutputDir)

		if err := verifyWindowsSignature(binary); err != nil {
			return err
		}
	}

	return nil
}

func resolveCodeSignToolPath(ctx *Context) (string, error) {
	if strings.TrimSpace(ctx.Env.CodeSignToolExe) != "" {
		path := filepath.Clean(ctx.Env.CodeSignToolExe)
		if !fileExists(path) {
			return "", fmt.Errorf("CodeSignTool not found at %s", path)
		}
		return path, nil
	}

	path := filepath.Join(ctx.Env.CodeSignToolPath, "CodeSignTool.bat")
	if !fileExists(path) {
		return "", fmt.Errorf("CodeSignTool not found at %s", path)
	}
	return path, nil
}

func runCodeSignCommand(toolPath, cwd string, args ...string) (cmdResult, error) {
	lower := strings.ToLower(toolPath)
	if strings.HasSuffix(lower, ".bat") || strings.HasSuffix(lower, ".cmd") {
		cmdArgs := append([]string{"/C", toolPath}, args...)
		return runCmdCapture(cwd, "cmd", cmdArgs...)
	}
	return runCmdCapture(cwd, toolPath, args...)
}

func verifyWindowsSignature(binaryPath string) error {
	escaped := strings.ReplaceAll(binaryPath, "'", "''")
	result, err := runCmdCapture("", "powershell", "-Command", fmt.Sprintf("(Get-AuthenticodeSignature '%s').Status", escaped))
	if err != nil {
		return err
	}
	if result.ExitCode != 0 {
		return fmt.Errorf("failed to verify signature for %s: %s", binaryPath, firstNonEmpty(result.Stderr, result.Stdout))
	}
	if !strings.Contains(result.Stdout, "Valid") {
		return fmt.Errorf("signature verification failed for %s: %s", binaryPath, strings.TrimSpace(result.Stdout))
	}
	return nil
}
