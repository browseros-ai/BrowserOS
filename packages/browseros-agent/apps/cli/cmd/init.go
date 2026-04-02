package cmd

import (
	"bufio"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"strings"
	"time"

	"browseros-cli/config"
	"browseros-cli/output"

	"github.com/fatih/color"
	"github.com/spf13/cobra"
)

func init() {
	var autoDiscover bool

	cmd := &cobra.Command{
		Use:   "init [url]",
		Short: "Configure the BrowserOS server connection",
		Long: `Set up the CLI by providing the MCP server URL from BrowserOS.

Open BrowserOS → Settings → BrowserOS MCP to find your Server URL.
The URL looks like: http://127.0.0.1:9000/mcp

The port varies per installation, so this step is required on first use.
Run again if your port changes.

You can provide the full URL or just the port number:
  browseros-cli init http://127.0.0.1:9000/mcp
  browseros-cli init 9000

Three modes:
  browseros-cli init <url>    Non-interactive (full URL or port number)
  browseros-cli init --auto   Auto-discover from ~/.browseros/server.json
  browseros-cli init          Interactive prompt`,
		Annotations: map[string]string{"group": "Setup:"},
		Args:        cobra.MaximumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			bold := color.New(color.Bold)
			green := color.New(color.FgGreen)
			dim := color.New(color.Faint)

			var input string

			switch {
			case len(args) == 1:
				// Non-interactive: URL provided as argument
				input = args[0]

			case autoDiscover:
				// Auto-discover: server.json → config → probe common ports
				discovered := probeRunningServer()
				if discovered == "" {
					output.Error("auto-discovery failed: no running BrowserOS found.\n\n"+
						"  If not running:    browseros-cli launch\n"+
						"  If not installed:  browseros-cli install", 1)
				}
				input = discovered
				fmt.Printf("Auto-discovered server at %s\n", input)

			default:
				// Interactive prompt (original behavior)
				fmt.Println()
				bold.Println("BrowserOS CLI Setup")
				fmt.Println()
				fmt.Println("Open BrowserOS → Settings → BrowserOS MCP")
				fmt.Println("Copy the Server URL or port number shown there.")
				fmt.Println()
				dim.Println("Examples:  http://127.0.0.1:9000/mcp")
				dim.Println("           9000")
				fmt.Println()

				reader := bufio.NewReader(os.Stdin)
				fmt.Print("Server URL or port: ")
				line, err := reader.ReadString('\n')
				if err != nil {
					output.Error("failed to read input", 1)
				}
				input = strings.TrimSpace(line)

				if input == "" {
					output.Error("no URL provided", 1)
				}
			}

			baseURL := normalizeServerURL(input)

			parsed, err := url.Parse(baseURL)
			if err != nil || parsed.Host == "" {
				output.Errorf(1, "invalid URL: %s", input)
			}

			// Verify connectivity
			fmt.Printf("Checking connection to %s ...\n", baseURL)
			client := &http.Client{Timeout: 5 * time.Second}
			resp, err := client.Get(baseURL + "/health")
			if err != nil {
				output.Errorf(1, "cannot connect to %s: %v\nIs BrowserOS running?", baseURL, err)
			}
			resp.Body.Close()

			if resp.StatusCode >= 400 {
				output.Errorf(1, "server returned HTTP %d — check the URL", resp.StatusCode)
			}

			cfg := &config.Config{ServerURL: baseURL}
			if err := config.Save(cfg); err != nil {
				output.Errorf(1, "save config: %v", err)
			}

			fmt.Println()
			green.Printf("Connected! Config saved to %s\n", config.Path())
			fmt.Println()
			dim.Println("Try: browseros-cli health")
			dim.Println("     browseros-cli pages")
		},
	}

	cmd.Flags().BoolVar(&autoDiscover, "auto", false, "Auto-discover server URL from ~/.browseros/server.json")
	rootCmd.AddCommand(cmd)
}
