package cmd

import (
	"fmt"
	"os"
	"path/filepath"
	"time"

	"bros/internal/config"
	"bros/internal/engine"
	"bros/internal/git"
	"bros/internal/log"
	"bros/internal/ui"

	"github.com/spf13/cobra"
)

var cloneCmd = &cobra.Command{
	Use:   "clone",
	Short: "Fresh-apply all patches (for CI/new checkouts)",
	Long: `Apply all patches from the patches repository onto the current
Chromium checkout. Used for CI builds and new checkout setup.
Unlike pull, clone does not compare existing state — it applies everything.`,
	RunE: runClone,
}

var (
	clonePatchesRepo string
	cloneVerifyBase  bool
	cloneClean       bool
	cloneDryRun      bool
	cloneName        string
)

func init() {
	cloneCmd.Flags().StringVar(&clonePatchesRepo, "patches-repo", "", "path to BrowserOS packages directory")
	cloneCmd.Flags().BoolVar(&cloneVerifyBase, "verify-base", false, "fail if HEAD != BASE_COMMIT")
	cloneCmd.Flags().BoolVar(&cloneClean, "clean", false, "reset all modified files to BASE before applying")
	cloneCmd.Flags().BoolVar(&cloneDryRun, "dry-run", false, "show what would be applied")
	cloneCmd.Flags().StringVar(&cloneName, "name", "", "checkout name (default: directory name)")
	rootCmd.AddCommand(cloneCmd)
}

func runClone(cmd *cobra.Command, args []string) error {
	cwd, err := os.Getwd()
	if err != nil {
		return fmt.Errorf("getting cwd: %w", err)
	}

	// Try loading existing context, or create one from flags
	ctx, err := config.LoadContext()
	if err != nil {
		// No existing .bros/ — need --patches-repo
		if clonePatchesRepo == "" {
			return fmt.Errorf("no .bros/ found and --patches-repo not specified")
		}

		patchesRepo, err := filepath.Abs(clonePatchesRepo)
		if err != nil {
			return fmt.Errorf("resolving patches repo: %w", err)
		}

		baseCommit, err := config.ReadBaseCommit(patchesRepo)
		if err != nil {
			return err
		}

		name := cloneName
		if name == "" {
			name = filepath.Base(cwd)
		}

		brosDir := filepath.Join(cwd, config.BrosDirName)
		cfg := &config.Config{
			Name:        name,
			PatchesRepo: patchesRepo,
		}
		if !cloneDryRun {
			if err := config.WriteConfig(brosDir, cfg); err != nil {
				return err
			}
			_ = os.MkdirAll(filepath.Join(brosDir, "logs"), 0o755)
		}

		chromiumVersion, _ := config.ReadChromiumVersion(patchesRepo)
		ctx = &config.Context{
			Config:          cfg,
			State:           &config.State{},
			ChromiumDir:     cwd,
			BrosDir:         brosDir,
			PatchesRepo:     patchesRepo,
			PatchesDir:      filepath.Join(patchesRepo, "chromium_patches"),
			BaseCommit:      baseCommit,
			ChromiumVersion: chromiumVersion,
		}
	}

	if cloneDryRun {
		fmt.Println(ui.MutedStyle.Render("dry run — no files will be modified"))
		fmt.Println()
	}

	opts := engine.CloneOpts{
		VerifyBase: cloneVerifyBase,
		Clean:      cloneClean,
		DryRun:     cloneDryRun,
	}

	result, err := engine.Clone(ctx, opts)
	if err != nil {
		return err
	}

	// Reuse pull rendering
	fmt.Println(ui.TitleStyle.Render("bros clone"))
	fmt.Println()
	fmt.Printf("  %s %d patches applied\n",
		ui.SuccessStyle.Render("+"), len(result.Applied))
	if len(result.Conflicts) > 0 {
		fmt.Printf("  %s %d conflicts\n",
			ui.ErrorStyle.Render("x"), len(result.Conflicts))
	}
	if len(result.Deleted) > 0 {
		fmt.Printf("  %s %d files deleted\n",
			ui.DeletedPrefix, len(result.Deleted))
	}

	if len(result.Conflicts) > 0 {
		fmt.Print(ui.RenderConflictReport(result.Conflicts))
	}

	if !cloneDryRun {
		repoRev, _ := git.HeadRev(ctx.PatchesRepo)
		ctx.State.LastPull = &config.SyncEvent{
			PatchesRepoRev: repoRev,
			Timestamp:      time.Now(),
			FileCount:      len(result.Applied),
		}
		_ = config.WriteState(ctx.BrosDir, ctx.State)

		logger := log.New(ctx.BrosDir)
		_ = logger.LogClone(ctx.BaseCommit, result)
	}

	if len(result.Conflicts) > 0 {
		return fmt.Errorf("%d conflicts — see above for details", len(result.Conflicts))
	}

	return nil
}
