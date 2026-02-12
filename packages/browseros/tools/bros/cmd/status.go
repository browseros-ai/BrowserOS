package cmd

import (
	"encoding/json"
	"fmt"

	"bros/internal/config"
	"bros/internal/engine"
	"bros/internal/ui"

	"github.com/spf13/cobra"
)

var statusCmd = &cobra.Command{
	Use:   "status",
	Short: "Show sync state between checkout and patches repo",
	RunE:  runStatus,
}

var (
	statusJSON  bool
	statusFiles bool
)

func init() {
	statusCmd.Flags().BoolVar(&statusJSON, "json", false, "output as JSON")
	statusCmd.Flags().BoolVar(&statusFiles, "files", false, "list individual files per category")
	rootCmd.AddCommand(statusCmd)
}

func runStatus(cmd *cobra.Command, args []string) error {
	ctx, err := config.LoadContext()
	if err != nil {
		return err
	}

	result, err := engine.Status(ctx, statusFiles)
	if err != nil {
		return err
	}

	if statusJSON {
		data, err := json.MarshalIndent(result, "", "  ")
		if err != nil {
			return err
		}
		fmt.Println(string(data))
		return nil
	}

	renderStatus(result)
	return nil
}

func renderStatus(r *engine.StatusResult) {
	fmt.Println(ui.TitleStyle.Render("bros status"))
	fmt.Println()

	fmt.Printf("  %s %s\n", ui.LabelStyle.Render("Checkout:"), ui.ValueStyle.Render(r.CheckoutName))
	fmt.Printf("  %s %s\n", ui.LabelStyle.Render("Base commit:"), r.BaseCommit[:min(12, len(r.BaseCommit))])
	if r.ChromiumVersion != "" {
		fmt.Printf("  %s %s\n", ui.LabelStyle.Render("Chromium:"), r.ChromiumVersion)
	}
	fmt.Printf("  %s %s\n", ui.LabelStyle.Render("Patches repo:"), r.PatchesRepo)

	fmt.Println()
	fmt.Println("  Sync status:")

	if r.Ahead > 0 {
		fmt.Printf("    %s %s\n",
			ui.WarningStyle.Render(fmt.Sprintf("ahead:  %3d files", r.Ahead)),
			ui.MutedStyle.Render("(local changes not in patches repo)"))
	}
	if r.Behind > 0 {
		fmt.Printf("    %s %s\n",
			ui.WarningStyle.Render(fmt.Sprintf("behind: %3d files", r.Behind)),
			ui.MutedStyle.Render("(patches in repo not applied locally)"))
	}
	fmt.Printf("    %s\n",
		ui.SuccessStyle.Render(fmt.Sprintf("synced: %3d files", r.Synced)))

	if len(r.AheadFiles) > 0 {
		fmt.Println()
		fmt.Println("  Ahead files:")
		for _, f := range r.AheadFiles {
			fmt.Printf("    %s %s\n", ui.AddedPrefix, f)
		}
	}
	if len(r.BehindFiles) > 0 {
		fmt.Println()
		fmt.Println("  Behind files:")
		for _, f := range r.BehindFiles {
			fmt.Printf("    %s %s\n", ui.WarningStyle.Render(">"), f)
		}
	}
}
