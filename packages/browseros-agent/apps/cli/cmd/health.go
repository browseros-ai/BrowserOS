package cmd

import (
	"fmt"

	"browseros-cli/output"

	"github.com/fatih/color"
	"github.com/spf13/cobra"
)

func init() {
	healthCmd := &cobra.Command{
		Use:         "health",
		Annotations: map[string]string{"group": "Setup:"},
		Short:       "Check BrowserOS server health",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			data, err := c.Health()
			if err != nil {
				output.Error(err.Error(), 2)
			}
			if jsonOut {
				output.JSONRaw(data)
				return
			}
			status := fmt.Sprintf("%v", data["status"])
			cdp := data["cdpConnected"]
			green := color.New(color.FgGreen).SprintFunc()
			red := color.New(color.FgRed).SprintFunc()

			statusStr := red(status)
			if status == "ok" {
				statusStr = green(status)
			}
			fmt.Printf("Server: %s\n", statusStr)

			if cdp != nil {
				cdpStr := red("disconnected")
				if b, ok := cdp.(bool); ok && b {
					cdpStr = green("connected")
				}
				fmt.Printf("CDP:    %s\n", cdpStr)
			}
		},
	}

	statusCmd := &cobra.Command{
		Use:         "status",
		Annotations: map[string]string{"group": "Setup:"},
		Short:       "Check BrowserOS runtime status",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			data, err := c.Status()
			if err != nil {
				output.Error(err.Error(), 2)
			}
			if jsonOut {
				output.JSONRaw(data)
				return
			}
			green := color.New(color.FgGreen).SprintFunc()
			red := color.New(color.FgRed).SprintFunc()

			cdp := data["cdpConnected"]
			cdpStr := red("disconnected")
			if b, ok := cdp.(bool); ok && b {
				cdpStr = green("connected")
			}
			fmt.Printf("Browser: %s\n", cdpStr)
		},
	}

	rootCmd.AddCommand(healthCmd, statusCmd)
}
