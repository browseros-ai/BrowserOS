package cmd

import (
	"fmt"
	"os"
	"path/filepath"

	"bros/internal/config"
	"bros/internal/git"
	"bros/internal/ui"

	"github.com/spf13/cobra"
)

var initCmd = &cobra.Command{
	Use:   "init",
	Short: "Initialize a Chromium checkout for bros",
	Long:  "Sets up a .bros/ directory in the current Chromium checkout,\nlinking it to a BrowserOS patches repository.",
	RunE:  runInit,
}

var (
	initPatchesRepo string
	initName        string
)

func init() {
	initCmd.Flags().StringVar(&initPatchesRepo, "patches-repo", "", "path to BrowserOS packages directory (required)")
	initCmd.Flags().StringVar(&initName, "name", "", "human name for this checkout (default: directory name)")
	_ = initCmd.MarkFlagRequired("patches-repo")
	rootCmd.AddCommand(initCmd)
}

func runInit(cmd *cobra.Command, args []string) error {
	cwd, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("getting cwd: %w", err)
	}

	if !config.LooksLikeChromium(cwd) {
		return fmt.Errorf("current directory does not look like a Chromium checkout (missing chrome/, base/, or .git/)")
	}

	brosDir := filepath.Join(cwd, config.BrosDirName)
	if _, err := os.Stat(filepath.Join(brosDir, "config.yaml")); err == nil {
		return fmt.Errorf(".bros/config.yaml already exists — checkout already initialized")
	}

	patchesRepo, err := filepath.Abs(initPatchesRepo)
	if err != nil {
		return fmt.Errorf("resolving patches repo path: %w", err)
	}

	patchesDir := filepath.Join(patchesRepo, "chromium_patches")
	if _, err := os.Stat(patchesDir); err != nil {
		return fmt.Errorf("chromium_patches/ not found in %s", patchesRepo)
	}

	baseCommit, err := config.ReadBaseCommit(patchesRepo)
	if err != nil {
		return err
	}

	if !git.CommitExists(cwd, baseCommit) {
		return fmt.Errorf("BASE_COMMIT %s not found in this checkout's git history", baseCommit)
	}

	name := initName
	if name == "" {
		name = filepath.Base(cwd)
	}

	cfg := &config.Config{
		Name:        name,
		PatchesRepo: patchesRepo,
	}
	if err := config.WriteConfig(brosDir, cfg); err != nil {
		return err
	}

	// Create logs directory
	if err := os.MkdirAll(filepath.Join(brosDir, "logs"), 0o755); err != nil {
		return fmt.Errorf("creating logs directory: %w", err)
	}

	chromiumVersion, _ := config.ReadChromiumVersion(patchesRepo)

	// Count existing patches
	patchCount := 0
	_ = filepath.Walk(patchesDir, func(path string, info os.FileInfo, err error) error {
		if err != nil {
			return nil
		}
		if !info.IsDir() {
			patchCount++
		}
		return nil
	})

	fmt.Println(ui.TitleStyle.Render("bros init"))
	fmt.Println()
	fmt.Printf("  %s %s\n", ui.LabelStyle.Render("Checkout:"), ui.ValueStyle.Render(name))
	fmt.Printf("  %s %s\n", ui.LabelStyle.Render("Directory:"), cwd)
	fmt.Printf("  %s %s\n", ui.LabelStyle.Render("Patches repo:"), patchesRepo)
	fmt.Printf("  %s %s\n", ui.LabelStyle.Render("Base commit:"), baseCommit[:min(12, len(baseCommit))])
	if chromiumVersion != "" {
		fmt.Printf("  %s %s\n", ui.LabelStyle.Render("Chromium:"), chromiumVersion)
	}
	fmt.Printf("  %s %d files\n", ui.LabelStyle.Render("Patches:"), patchCount)
	fmt.Println()
	fmt.Println(ui.SuccessStyle.Render("Initialized .bros/config.yaml"))
	fmt.Println(ui.MutedStyle.Render("Run 'bros pull' to apply patches, or 'bros push' to extract."))

	return nil
}
