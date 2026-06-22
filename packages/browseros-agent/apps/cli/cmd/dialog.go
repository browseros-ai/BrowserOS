package cmd

import (
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "dialog <accept|dismiss>",
		Annotations: map[string]string{"group": "Input:"},
		Short:       "Handle a JavaScript dialog",
		Args:        cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			action := args[0]
			if action != "accept" && action != "dismiss" {
				output.Errorf(3, "action must be 'accept' or 'dismiss', got: %s", action)
			}

			promptText, _ := cmd.Flags().GetString("text")

			toolArgs := map[string]any{
				"accept": action == "accept",
			}
			if promptText != "" {
				toolArgs["promptText"] = promptText
			}

			_ = toolArgs
			unsupportedByCurrentMCP("dialog")
		},
	}

	cmd.Flags().String("text", "", "Text for prompt dialogs")
	rootCmd.AddCommand(cmd)
}
