package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/ui"
	"github.com/spf13/cobra"
)

func init() {
	var src string
	var commit string
	var rangeMode bool
	var squash bool
	var base string
	command := &cobra.Command{
		Use:         "extract [checkout] [--range <start> <end>] [-- files...]",
		Annotations: map[string]string{"group": "Core:"},
		Short:       "Extract checkout changes back to chromium_patches",
		Example: `  browseros-patch extract ch1
  browseros-patch extract ch1 --range HEAD~2 HEAD
  browseros-patch extract --src /path/to/chromium/src`,
		Args: cobra.ArbitraryArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			positional, filters := splitWorkspaceAndFilters(cmd, args)
			workspaceArgs := positional
			rangeStart := ""
			rangeEnd := ""
			if rangeMode {
				if len(positional) < 2 || len(positional) > 3 {
					return fmt.Errorf(`range mode expects "browseros-patch extract [checkout] --range <start> <end>"`)
				}
				rangeStart = positional[len(positional)-2]
				rangeEnd = positional[len(positional)-1]
				workspaceArgs = positional[:len(positional)-2]
			}
			if len(workspaceArgs) > 1 {
				return fmt.Errorf("expected at most one checkout name")
			}
			ws, err := resolveWorkspace(cmd, workspaceArgs, src)
			if err != nil {
				return err
			}
			info, err := repoInfo()
			if err != nil {
				return err
			}
			result, err := engine.Extract(cmd.Context(), engine.ExtractOptions{
				Workspace:  ws,
				Repo:       info,
				Commit:     commit,
				RangeStart: rangeStart,
				RangeEnd:   rangeEnd,
				Squash:     squash,
				Base:       base,
				Filters:    filters,
				Progress:   commandProgress(cmd),
			})
			if err != nil {
				return err
			}
			return renderResult(result, func() {
				fmt.Println(ui.Title(fmt.Sprintf("Extracted patches from %s", ws.Name)))
				fmt.Printf("%s  %s\n", ui.Muted("mode:"), result.Mode)
				fmt.Printf("%s  %d\n", ui.Muted("written:"), len(result.Written))
				fmt.Printf("%s  %d\n", ui.Muted("deleted:"), len(result.Deleted))
			})
		},
	}
	command.Flags().StringVar(&src, "src", "", srcFlagUsage)
	command.Flags().StringVar(&commit, "commit", "", "Extract from a single commit")
	command.Flags().BoolVar(&rangeMode, "range", false, "Extract from a commit range")
	command.Flags().BoolVar(&squash, "squash", false, "Squash a range into a cumulative diff")
	command.Flags().StringVar(&base, "base", "", "Override BASE_COMMIT for extraction")
	rootCmd.AddCommand(command)
}
