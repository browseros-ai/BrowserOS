package cmd

import (
	"fmt"

	"bros/internal/config"
	"bros/internal/git"
	"bros/internal/patch"
	"bros/internal/ui"

	"github.com/spf13/cobra"
)

var diffCmd = &cobra.Command{
	Use:   "diff",
	Short: "Preview what push or pull would do",
	RunE:  runDiff,
}

var diffDirection string

func init() {
	diffCmd.Flags().StringVar(&diffDirection, "direction", "push", "\"push\" or \"pull\"")
	rootCmd.AddCommand(diffCmd)
}

func runDiff(cmd *cobra.Command, args []string) error {
	ctx, err := config.LoadContext()
	if err != nil {
		return err
	}

	switch diffDirection {
	case "push":
		return diffPush(ctx)
	case "pull":
		return diffPull(ctx)
	default:
		return fmt.Errorf("invalid direction %q — use \"push\" or \"pull\"", diffDirection)
	}
}

func diffPush(ctx *config.Context) error {
	nameStatus, err := git.DiffNameStatus(ctx.ChromiumDir, ctx.BaseCommit)
	if err != nil {
		return err
	}

	if len(nameStatus) == 0 {
		fmt.Println(ui.MutedStyle.Render("No local changes to push."))
		return nil
	}

	fmt.Println(ui.TitleStyle.Render("bros diff --direction push"))
	fmt.Println()

	for path, op := range nameStatus {
		prefix := ui.ModifiedPrefix
		switch op {
		case patch.OpAdded:
			prefix = ui.AddedPrefix
		case patch.OpDeleted:
			prefix = ui.DeletedPrefix
		}
		fmt.Printf("  %s %s\n", prefix, path)
	}
	fmt.Println()
	fmt.Println(ui.MutedStyle.Render(fmt.Sprintf("%d files would be pushed", len(nameStatus))))

	return nil
}

func diffPull(ctx *config.Context) error {
	repoPatchSet, err := patch.ReadPatchSet(ctx.PatchesDir)
	if err != nil {
		return err
	}

	diffOutput, err := git.DiffFull(ctx.ChromiumDir, ctx.BaseCommit)
	if err != nil {
		return err
	}

	localPatchSet, err := patch.ParseUnifiedDiff(diffOutput)
	if err != nil {
		return err
	}

	delta := patch.Compare(localPatchSet, repoPatchSet)

	total := len(delta.NeedsUpdate) + len(delta.NeedsApply)
	if total == 0 && len(delta.Deleted) == 0 {
		fmt.Println(ui.MutedStyle.Render("Already up to date."))
		return nil
	}

	fmt.Println(ui.TitleStyle.Render("bros diff --direction pull"))
	fmt.Println()

	for _, f := range delta.NeedsUpdate {
		fmt.Printf("  %s %s %s\n", ui.ModifiedPrefix, f, ui.MutedStyle.Render("(update)"))
	}
	for _, f := range delta.NeedsApply {
		fmt.Printf("  %s %s %s\n", ui.AddedPrefix, f, ui.MutedStyle.Render("(new)"))
	}
	for _, f := range delta.Deleted {
		fmt.Printf("  %s %s %s\n", ui.DeletedPrefix, f, ui.MutedStyle.Render("(delete)"))
	}

	fmt.Println()
	fmt.Println(ui.MutedStyle.Render(fmt.Sprintf("%d files would be changed", total+len(delta.Deleted))))

	return nil
}
