package cmd

import (
	"fmt"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	windowCmd := &cobra.Command{
		Use:         "window",
		Annotations: map[string]string{"group": "Resources:"},
		Short:       "Manage browser windows",
	}

	listCmd := &cobra.Command{
		Use:   "list",
		Short: "List all browser windows",
		Args:  cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("windows", map[string]any{"action": "list"})
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.Text(result)
			}
		},
	}

	createCmd := &cobra.Command{
		Use:   "create",
		Short: "Create a new browser window",
		Args:  cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			hidden, _ := cmd.Flags().GetBool("hidden")

			c := newClient()
			result, err := c.CallTool("windows", map[string]any{
				"action": "create",
				"hidden": hidden,
			})
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm(result.TextContent())
			}
		},
	}
	createCmd.Flags().Bool("hidden", false, "Create hidden window")

	closeCmd := &cobra.Command{
		Use:   "close <windowId>",
		Short: "Close a browser window",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			var windowID int
			if _, err := fmt.Sscanf(args[0], "%d", &windowID); err != nil {
				output.Errorf(3, "invalid window ID: %s", args[0])
			}
			c := newClient()
			result, err := c.CallTool("windows", map[string]any{
				"action":   "close",
				"windowId": windowID,
			})
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm(result.TextContent())
			}
		},
	}

	activateCmd := &cobra.Command{
		Use:   "activate <windowId>",
		Short: "Activate (focus) a browser window",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			var windowID int
			if _, err := fmt.Sscanf(args[0], "%d", &windowID); err != nil {
				output.Errorf(3, "invalid window ID: %s", args[0])
			}
			c := newClient()
			result, err := c.CallTool("windows", map[string]any{
				"action":   "activate",
				"windowId": windowID,
			})
			if err != nil {
				output.Error(err.Error(), 1)
			}
			if jsonOut {
				output.JSON(result)
			} else {
				output.Confirm(result.TextContent())
			}
		},
	}

	windowCmd.AddCommand(listCmd, createCmd, closeCmd, activateCmd)
	rootCmd.AddCommand(windowCmd)
}
