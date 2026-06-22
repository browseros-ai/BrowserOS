package cmd

import "github.com/spf13/cobra"

func init() {
	historyCmd := &cobra.Command{
		Use:         "history",
		Annotations: map[string]string{"group": "Resources:"},
		Short:       "Manage browser history",
	}

	searchCmd := &cobra.Command{
		Use:   "search <query>",
		Short: "Search browser history",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			max, _ := cmd.Flags().GetInt("max")
			toolArgs := map[string]any{"query": args[0]}
			if cmd.Flags().Changed("max") {
				toolArgs["maxResults"] = max
			}
			_ = toolArgs
			unsupportedByCurrentMCP("history search")
		},
	}
	searchCmd.Flags().Int("max", 0, "Max results")

	recentCmd := &cobra.Command{
		Use:   "recent",
		Short: "Show recent history",
		Args:  cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			max, _ := cmd.Flags().GetInt("max")
			toolArgs := map[string]any{}
			if cmd.Flags().Changed("max") {
				toolArgs["maxResults"] = max
			}
			_ = toolArgs
			unsupportedByCurrentMCP("history recent")
		},
	}
	recentCmd.Flags().Int("max", 0, "Max results")

	deleteCmd := &cobra.Command{
		Use:   "delete <url>",
		Short: "Delete a URL from history",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			unsupportedByCurrentMCP("history delete")
		},
	}

	deleteRangeCmd := &cobra.Command{
		Use:   "delete-range",
		Short: "Delete history within a time range",
		Args:  cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			start, _ := cmd.Flags().GetInt("start")
			end, _ := cmd.Flags().GetInt("end")
			toolArgs := map[string]any{
				"startTime": start,
				"endTime":   end,
			}
			_ = toolArgs
			unsupportedByCurrentMCP("history delete-range")
		},
	}
	deleteRangeCmd.Flags().Int("start", 0, "Start time (epoch ms)")
	deleteRangeCmd.Flags().Int("end", 0, "End time (epoch ms)")
	_ = deleteRangeCmd.MarkFlagRequired("start")
	_ = deleteRangeCmd.MarkFlagRequired("end")

	historyCmd.AddCommand(searchCmd, recentCmd, deleteCmd, deleteRangeCmd)
	rootCmd.AddCommand(historyCmd)
}
