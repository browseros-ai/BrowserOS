package cmd

import (
	"context"
	"fmt"
	"strings"

	nativecommon "bros/internal/native/common"
	nativeota "bros/internal/native/ota"
	nativer2 "bros/internal/native/r2"
	nativerelease "bros/internal/native/release"

	"github.com/spf13/cobra"
)

type releaseCompatOptions struct {
	Version     string
	List        bool
	Appcast     bool
	Publish     bool
	Download    bool
	OSFilter    string
	Output      string
	ShowModules bool
}

var releaseCompatOpts releaseCompatOptions

var releaseCmd = &cobra.Command{
	Use:   "release",
	Short: "Release automation commands",
	RunE: func(cmd *cobra.Command, args []string) error {
		return runReleaseCompat(releaseCompatOpts)
	},
}

func init() {
	releaseCmd.Flags().StringVarP(&releaseCompatOpts.Version, "version", "v", "", "version to operate on")
	releaseCmd.Flags().BoolVarP(&releaseCompatOpts.List, "list", "l", false, "list available artifacts")
	releaseCmd.Flags().BoolVarP(&releaseCompatOpts.Appcast, "appcast", "a", false, "generate appcast XML")
	releaseCmd.Flags().BoolVarP(&releaseCompatOpts.Publish, "publish", "p", false, "publish to download paths")
	releaseCmd.Flags().BoolVarP(&releaseCompatOpts.Download, "download", "d", false, "download artifacts")
	releaseCmd.Flags().StringVar(&releaseCompatOpts.OSFilter, "os", "", "filter by OS (macos, windows, linux)")
	releaseCmd.Flags().StringVarP(&releaseCompatOpts.Output, "output", "o", "", "download output directory")
	releaseCmd.Flags().BoolVar(&releaseCompatOpts.ShowModules, "show-modules", false, "show available modules")

	releaseCmd.AddCommand(
		newReleaseListCommand(),
		newReleaseAppcastCommand(),
		newReleasePublishCommand(),
		newReleaseDownloadCommand(),
		newReleaseGithubCommand(),
		newOTACommand("ota", "OTA update automation"),
	)

	rootCmd.AddCommand(releaseCmd)
	rootCmd.AddCommand(newOTACommand("ota", "OTA update automation (alias of bros release ota)"))
}

func runReleaseCompat(opts releaseCompatOptions) error {
	client, err := newReleaseClient()
	if err != nil {
		return err
	}

	result, err := nativerelease.Run(context.Background(), client, nativerelease.Options{
		Version:     opts.Version,
		List:        opts.List,
		Appcast:     opts.Appcast,
		Publish:     opts.Publish,
		Download:    opts.Download,
		OSFilter:    opts.OSFilter,
		Output:      opts.Output,
		ShowModules: opts.ShowModules,
	})
	if err != nil {
		return err
	}

	renderReleaseResult(opts, result)
	return nil
}

func newReleaseClient() (*nativer2.Client, error) {
	packagesDir, err := nativecommon.ResolveBrowserOSPackagesDir()
	if err != nil {
		return nil, err
	}
	_ = nativecommon.LoadEnv(packagesDir)
	return nativer2.NewClientFromEnv()
}

func renderReleaseResult(opts releaseCompatOptions, result nativerelease.RunResult) {
	if len(result.Modules) > 0 {
		fmt.Println("Available release modules:")
		for _, m := range result.Modules {
			fmt.Printf("  %s: %s\n", m.Name, m.Description)
		}
		return
	}

	if opts.List {
		if opts.Version == "" {
			if len(result.Versions) == 0 {
				fmt.Println("No releases found in R2")
				return
			}
			fmt.Printf("Available releases (%d total):\n", len(result.Versions))
			for _, version := range result.Versions {
				fmt.Printf("  %s\n", version)
			}
		} else {
			renderVersionMetadata(opts.Version, result.Metadata)
		}
	}

	if opts.Appcast && len(result.Appcast) > 0 {
		fmt.Println()
		fmt.Printf("APPCAST SNIPPETS FOR v%s\n", opts.Version)
		for _, snippet := range result.Appcast {
			fmt.Println()
			fmt.Printf("%s (%s):\n", snippet.Filename, snippet.Arch)
			fmt.Println(snippet.ItemXML)
		}
	}

	if opts.Publish {
		fmt.Println()
		success := 0
		for _, r := range result.PublishResults {
			if r.Err == nil {
				success++
				fmt.Printf("✓ %s -> %s\n", r.Filename, r.DestinationKey)
			} else {
				fmt.Printf("✗ %s -> %s (%v)\n", r.Filename, r.DestinationKey, r.Err)
			}
		}
		fmt.Printf("Published %d/%d artifacts\n", success, len(result.PublishResults))
	}

	if opts.Download {
		fmt.Println()
		for _, r := range result.Download.Results {
			if r.Err == nil {
				fmt.Printf("✓ %s (%s)\n", r.Filename, nativerelease.FormatSize(r.BytesWritten))
			} else {
				fmt.Printf("✗ %s (%v)\n", r.Filename, r.Err)
			}
		}
		if strings.TrimSpace(result.Download.Directory) != "" {
			fmt.Printf("Downloaded to: %s\n", result.Download.Directory)
		}
	}
}

func renderVersionMetadata(version string, metadata map[string]nativerelease.PlatformMetadata) {
	if len(metadata) == 0 {
		fmt.Printf("No release metadata found for version %s\n", version)
		return
	}

	fmt.Printf("Release: v%s\n", version)
	for _, platform := range nativerelease.Platforms {
		release, ok := metadata[platform]
		if !ok {
			continue
		}
		fmt.Printf("\n%s:\n", nativerelease.PlatformDisplayNames[platform])
		fmt.Printf("  Build Date: %s\n", release.BuildDate)
		fmt.Printf("  Chromium: %s\n", release.ChromiumVersion)
		if platform == nativerelease.PlatformMacOS && release.SparkleVersion != "" {
			fmt.Printf("  Sparkle Version: %s\n", release.SparkleVersion)
		}
		keys := make([]string, 0, len(release.Artifacts))
		for key := range release.Artifacts {
			keys = append(keys, key)
		}
		// deterministic display
		for _, key := range keys {
			artifact := release.Artifacts[key]
			size := nativerelease.FormatSize(artifact.Size)
			sig := ""
			if artifact.SparkleSignature != "" {
				sig = " [signed]"
			}
			fmt.Printf("  - %s: %s (%s)%s\n", key, artifact.Filename, size, sig)
			if artifact.URL != "" {
				fmt.Printf("    %s\n", artifact.URL)
			}
		}
	}
}

func newReleaseListCommand() *cobra.Command {
	var version string

	cmd := &cobra.Command{
		Use:   "list",
		Short: "List release versions or artifacts",
		RunE: func(cmd *cobra.Command, args []string) error {
			return runReleaseCompat(releaseCompatOptions{
				Version: version,
				List:    true,
			})
		},
	}

	cmd.Flags().StringVarP(&version, "version", "v", "", "version to list artifacts for")
	return cmd
}

func newReleaseAppcastCommand() *cobra.Command {
	var version string

	cmd := &cobra.Command{
		Use:   "appcast",
		Short: "Generate appcast XML snippets",
		RunE: func(cmd *cobra.Command, args []string) error {
			return runReleaseCompat(releaseCompatOptions{
				Version: version,
				Appcast: true,
			})
		},
	}

	cmd.Flags().StringVarP(&version, "version", "v", "", "version to generate appcast for")
	_ = cmd.MarkFlagRequired("version")
	return cmd
}

func newReleasePublishCommand() *cobra.Command {
	var version string

	cmd := &cobra.Command{
		Use:   "publish",
		Short: "Publish release artifacts to download paths",
		RunE: func(cmd *cobra.Command, args []string) error {
			return runReleaseCompat(releaseCompatOptions{
				Version: version,
				Publish: true,
			})
		},
	}

	cmd.Flags().StringVarP(&version, "version", "v", "", "version to publish")
	_ = cmd.MarkFlagRequired("version")
	return cmd
}

func newReleaseDownloadCommand() *cobra.Command {
	var version string
	var osFilter string
	var output string

	cmd := &cobra.Command{
		Use:   "download",
		Short: "Download release artifacts",
		RunE: func(cmd *cobra.Command, args []string) error {
			return runReleaseCompat(releaseCompatOptions{
				Version:  version,
				Download: true,
				OSFilter: osFilter,
				Output:   output,
			})
		},
	}

	cmd.Flags().StringVarP(&version, "version", "v", "", "version to download")
	cmd.Flags().StringVar(&osFilter, "os", "", "filter by OS (macos, windows, linux)")
	cmd.Flags().StringVarP(&output, "output", "o", "", "download output directory")
	_ = cmd.MarkFlagRequired("version")
	return cmd
}

func newReleaseGithubCommand() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "github",
		Short: "GitHub release operations",
		Run: func(cmd *cobra.Command, args []string) {
			_ = cmd.Help()
		},
	}

	var (
		version           string
		repo              string
		title             string
		skipUpload        bool
		noDraft           bool
		publishToDownload bool
	)

	createCmd := &cobra.Command{
		Use:   "create",
		Short: "Create GitHub release from R2 artifacts",
		RunE: func(cmd *cobra.Command, args []string) error {
			client, err := newReleaseClient()
			if err != nil {
				return err
			}
			result, err := nativerelease.CreateAndUploadGitHubRelease(context.Background(), client, nativerelease.GitHubReleaseOptions{
				Version:    version,
				Repo:       repo,
				Title:      title,
				Draft:      !noDraft,
				SkipUpload: skipUpload,
			})
			if err != nil {
				return err
			}

			if result.ReleaseExisted {
				fmt.Printf("Release v%s already exists\n", result.TagVersion)
			} else if strings.TrimSpace(result.ReleaseURL) != "" {
				fmt.Printf("Release created: %s\n", strings.TrimSpace(result.ReleaseURL))
			} else {
				fmt.Printf("Release v%s created\n", result.TagVersion)
			}

			if len(result.UploadResults) > 0 {
				for _, upload := range result.UploadResults {
					if upload.Err == nil {
						fmt.Printf("✓ Uploaded %s\n", upload.Filename)
					} else {
						fmt.Printf("✗ Failed %s: %v\n", upload.Filename, upload.Err)
					}
				}
			}

			if len(result.Appcast) > 0 {
				fmt.Println()
				fmt.Println("APPCAST SNIPPETS:")
				for _, snippet := range result.Appcast {
					fmt.Printf("\n%s (%s):\n%s\n", snippet.Filename, snippet.Arch, snippet.ItemXML)
				}
			}

			if publishToDownload {
				return runReleaseCompat(releaseCompatOptions{
					Version: version,
					Publish: true,
				})
			}
			return nil
		},
	}

	createCmd.Flags().StringVarP(&version, "version", "v", "", "version to release (e.g., 0.31.0)")
	createCmd.Flags().BoolVar(&noDraft, "no-draft", false, "create published release instead of draft")
	createCmd.Flags().StringVarP(&repo, "repo", "r", "", "GitHub repo (owner/name)")
	createCmd.Flags().BoolVar(&skipUpload, "skip-upload", false, "skip uploading artifacts to GitHub")
	createCmd.Flags().StringVarP(&title, "title", "t", "", "release title (default: v{version})")
	createCmd.Flags().BoolVarP(&publishToDownload, "publish", "p", false, "also publish to download/ paths after creating release")
	_ = createCmd.MarkFlagRequired("version")

	cmd.AddCommand(createCmd)
	return cmd
}

func newOTACommand(use string, short string) *cobra.Command {
	otaCmd := &cobra.Command{
		Use:   use,
		Short: short,
	}

	testSigningCmd := &cobra.Command{
		Use:   "test-signing <file>",
		Short: "Test Sparkle Ed25519 signing on a file",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			return runOTACommand([]string{"ota", "test-signing", args[0]})
		},
	}

	serverCmd := &cobra.Command{
		Use:   "server",
		Short: "BrowserOS Server OTA commands",
		Run: func(cmd *cobra.Command, args []string) {
			_ = cmd.Help()
		},
	}

	var (
		releaseVersion  string
		releaseChannel  string
		releaseBinaries string
		releasePlatform string
	)
	serverReleaseCmd := &cobra.Command{
		Use:   "release",
		Short: "Create and upload server OTA artifacts",
		RunE: func(cmd *cobra.Command, args []string) error {
			argv := []string{"ota", "server", "release", "--version", releaseVersion}
			if strings.TrimSpace(releaseChannel) != "" {
				argv = append(argv, "--channel", releaseChannel)
			}
			if strings.TrimSpace(releaseBinaries) != "" {
				argv = append(argv, "--binaries", releaseBinaries)
			}
			if strings.TrimSpace(releasePlatform) != "" {
				argv = append(argv, "--platform", releasePlatform)
			}
			return runOTACommand(argv)
		},
	}
	serverReleaseCmd.Flags().StringVarP(&releaseVersion, "version", "v", "", "version to release")
	serverReleaseCmd.Flags().StringVarP(&releaseChannel, "channel", "c", "alpha", "release channel: alpha or prod")
	serverReleaseCmd.Flags().StringVarP(&releaseBinaries, "binaries", "b", "", "directory containing server binaries")
	serverReleaseCmd.Flags().StringVarP(&releasePlatform, "platform", "p", "", "platform(s) to process, comma-separated")
	_ = serverReleaseCmd.MarkFlagRequired("version")

	var (
		publishChannel string
		publishFile    string
	)
	serverPublishAppcastCmd := &cobra.Command{
		Use:     "publish-appcast",
		Aliases: []string{"release-appcast"},
		Short:   "Publish OTA appcast to make release live",
		RunE: func(cmd *cobra.Command, args []string) error {
			argv := []string{"ota", "server", "publish-appcast"}
			if strings.TrimSpace(publishChannel) != "" {
				argv = append(argv, "--channel", publishChannel)
			}
			if strings.TrimSpace(publishFile) != "" {
				argv = append(argv, "--file", publishFile)
			}
			return runOTACommand(argv)
		},
	}
	serverPublishAppcastCmd.Flags().StringVarP(&publishChannel, "channel", "c", "alpha", "release channel: alpha or prod")
	serverPublishAppcastCmd.Flags().StringVarP(&publishFile, "file", "f", "", "custom appcast file to upload")

	serverListPlatformsCmd := &cobra.Command{
		Use:   "list-platforms",
		Short: "List supported OTA server platforms",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runOTACommand([]string{"ota", "server", "list-platforms"})
		},
	}

	serverCmd.AddCommand(serverReleaseCmd, serverPublishAppcastCmd, serverListPlatformsCmd)
	otaCmd.AddCommand(testSigningCmd, serverCmd)

	return otaCmd
}

func runOTACommand(args []string) error {
	packagesDir, err := nativecommon.ResolveBrowserOSPackagesDir()
	if err != nil {
		return err
	}
	handled, err := nativeota.Run(args, packagesDir)
	if !handled {
		return fmt.Errorf("unsupported ota command")
	}
	return err
}
