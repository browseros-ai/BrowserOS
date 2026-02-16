package ota

import (
	"context"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sort"
	"strings"

	"bros/internal/native/common"
)

type serverReleaseOptions struct {
	Version      string
	Channel      string
	BinariesDir  string
	PlatformList string
}

func runServerRelease(args []string, packagesDir string) error {
	opts, err := parseServerReleaseOptions(args)
	if err != nil {
		return err
	}

	env := common.LoadEnv(packagesDir)
	if runtime.GOOS == "darwin" && env.MacOSCertificateName == "" {
		return fmt.Errorf("MACOS_CERTIFICATE_NAME required for signing")
	}
	if runtime.GOOS == "windows" && env.CodeSignToolPath == "" && env.CodeSignToolExe == "" {
		return fmt.Errorf("CODE_SIGN_TOOL_PATH or CODE_SIGN_TOOL_EXE required for signing on Windows")
	}
	if !env.HasR2Config() {
		return fmt.Errorf("R2 configuration not set. Required env vars: R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY")
	}

	platforms, err := platformsFromFilter(opts.PlatformList)
	if err != nil {
		return err
	}
	if len(platforms) == 0 {
		return fmt.Errorf("no platforms selected")
	}

	binariesDir := opts.BinariesDir
	if binariesDir == "" {
		binariesDir = filepath.Join(packagesDir, "resources", "binaries", "browseros_server")
	}

	if err := validateReleaseInputs(binariesDir, platforms); err != nil {
		return err
	}

	tempDir, err := os.MkdirTemp("", "bros-server-ota-*")
	if err != nil {
		return fmt.Errorf("creating temporary directory: %w", err)
	}
	defer os.RemoveAll(tempDir)

	repoRoot := filepath.Clean(packagesDir)
	signedArtifacts := make([]SignedArtifact, 0, len(platforms))

	for _, platform := range platforms {
		fmt.Printf("Processing %s...\n", platform.Name)

		sourceBinary := filepath.Join(binariesDir, platform.Binary)
		tempBinary := filepath.Join(tempDir, platform.Binary)
		if err := copyFile(sourceBinary, tempBinary); err != nil {
			fmt.Printf("  Skipping %s: failed to copy binary: %v\n", platform.Name, err)
			continue
		}

		if err := signBinary(tempBinary, platform, env, repoRoot); err != nil {
			fmt.Printf("  Skipping %s: signing failed: %v\n", platform.Name, err)
			continue
		}

		zipName := fmt.Sprintf("browseros_server_%s_%s.zip", opts.Version, platform.Name)
		zipPath := filepath.Join(tempDir, zipName)
		if err := createServerZip(tempBinary, zipPath, platform.OS == "windows"); err != nil {
			fmt.Printf("  Skipping %s: failed to create zip: %v\n", platform.Name, err)
			continue
		}

		signature, length, err := signWithSparkle(zipPath, env)
		if err != nil {
			fmt.Printf("  Skipping %s: Sparkle signing failed: %v\n", platform.Name, err)
			continue
		}

		signedArtifacts = append(signedArtifacts, SignedArtifact{
			Platform:  platform.Name,
			ZipPath:   zipPath,
			Signature: signature,
			Length:    length,
			OS:        platform.OS,
			Arch:      platform.Arch,
		})

		fmt.Printf("  %s complete (%d bytes)\n", platform.Name, length)
	}

	if len(signedArtifacts) == 0 {
		return fmt.Errorf("no artifacts were processed successfully")
	}

	appcastFile := appcastPath(packagesDir, opts.Channel)
	existingAppcast, err := parseExistingAppcast(appcastFile)
	if err != nil {
		fmt.Printf("Warning: failed to parse existing appcast %s: %v\n", appcastFile, err)
	}

	appcastContent := generateServerAppcast(opts.Version, signedArtifacts, opts.Channel, existingAppcast)
	if err := os.MkdirAll(filepath.Dir(appcastFile), 0o755); err != nil {
		return fmt.Errorf("creating appcast directory: %w", err)
	}
	if err := os.WriteFile(appcastFile, []byte(appcastContent), 0o644); err != nil {
		return fmt.Errorf("writing appcast file: %w", err)
	}
	fmt.Printf("Appcast saved: %s\n", appcastFile)

	r2Client, err := common.NewR2Client(env)
	if err != nil {
		return err
	}

	for _, artifact := range signedArtifacts {
		r2Key := "server/" + filepath.Base(artifact.ZipPath)
		fmt.Printf("Uploading %s...\n", r2Key)
		if err := r2Client.UploadFile(context.Background(), artifact.ZipPath, r2Key); err != nil {
			return fmt.Errorf("failed to upload %s: %w", r2Key, err)
		}
	}

	sort.Slice(signedArtifacts, func(i, j int) bool {
		return signedArtifacts[i].Platform < signedArtifacts[j].Platform
	})

	fmt.Println("Release artifacts ready:")
	for _, artifact := range signedArtifacts {
		fmt.Printf("  https://cdn.browseros.com/server/%s\n", filepath.Base(artifact.ZipPath))
	}
	fmt.Printf("\nNext step: bros ota server publish-appcast --channel %s\n", opts.Channel)

	return nil
}

func parseServerReleaseOptions(args []string) (serverReleaseOptions, error) {
	opts := serverReleaseOptions{}

	fs := flag.NewFlagSet("server-release", flag.ContinueOnError)
	fs.SetOutput(os.Stderr)
	fs.StringVar(&opts.Version, "version", "", "version to release")
	fs.StringVar(&opts.Version, "v", "", "version to release")
	fs.StringVar(&opts.Channel, "channel", "alpha", "release channel: alpha or prod")
	fs.StringVar(&opts.Channel, "c", "alpha", "release channel: alpha or prod")
	fs.StringVar(&opts.BinariesDir, "binaries", "", "directory containing server binaries")
	fs.StringVar(&opts.BinariesDir, "b", "", "directory containing server binaries")
	fs.StringVar(&opts.PlatformList, "platform", "", "platform(s) to process, comma-separated")
	fs.StringVar(&opts.PlatformList, "p", "", "platform(s) to process, comma-separated")

	if err := fs.Parse(args); err != nil {
		return serverReleaseOptions{}, err
	}

	if strings.TrimSpace(opts.Version) == "" {
		return serverReleaseOptions{}, fmt.Errorf("--version is required")
	}
	opts.Version = strings.TrimSpace(opts.Version)
	opts.Channel = strings.TrimSpace(opts.Channel)
	if opts.Channel == "" {
		opts.Channel = "alpha"
	}
	if opts.Channel != "alpha" && opts.Channel != "prod" {
		return serverReleaseOptions{}, fmt.Errorf("channel must be 'alpha' or 'prod'")
	}
	opts.BinariesDir = strings.TrimSpace(opts.BinariesDir)
	opts.PlatformList = strings.TrimSpace(opts.PlatformList)

	if fs.NArg() != 0 {
		return serverReleaseOptions{}, fmt.Errorf("unexpected positional arguments: %s", strings.Join(fs.Args(), " "))
	}

	return opts, nil
}

func platformsFromFilter(filter string) ([]ServerPlatform, error) {
	if strings.TrimSpace(filter) == "" {
		return append([]ServerPlatform(nil), serverPlatforms...), nil
	}

	parts := strings.Split(filter, ",")
	platforms := make([]ServerPlatform, 0, len(parts))
	seen := make(map[string]bool, len(parts))
	for _, part := range parts {
		name := strings.TrimSpace(part)
		if name == "" || seen[name] {
			continue
		}
		platform, ok := platformByName(name)
		if !ok {
			return nil, fmt.Errorf("unknown platform: %s", name)
		}
		seen[name] = true
		platforms = append(platforms, platform)
	}

	return platforms, nil
}

func validateReleaseInputs(binariesDir string, platforms []ServerPlatform) error {
	info, err := os.Stat(binariesDir)
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("binaries directory not found: %s", binariesDir)
		}
		return fmt.Errorf("checking binaries directory %s: %w", binariesDir, err)
	}
	if !info.IsDir() {
		return fmt.Errorf("binaries path is not a directory: %s", binariesDir)
	}

	for _, platform := range platforms {
		binaryPath := filepath.Join(binariesDir, platform.Binary)
		if _, err := os.Stat(binaryPath); err != nil {
			if os.IsNotExist(err) {
				return fmt.Errorf("binary not found: %s", binaryPath)
			}
			return fmt.Errorf("checking binary %s: %w", binaryPath, err)
		}
	}

	return nil
}
