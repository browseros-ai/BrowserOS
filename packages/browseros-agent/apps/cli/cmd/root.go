package cmd

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"

	"browseros-cli/analytics"
	"browseros-cli/config"
	"browseros-cli/mcp"
	"browseros-cli/output"
	"browseros-cli/update"

	"github.com/fatih/color"
	"github.com/spf13/cobra"
)

var (
	serverURL string
	pageFlag  int
	pageSet   bool
	jsonOut   bool
	debug     bool
	timeout   time.Duration
	version   = "dev"
)

const automaticUpdateDrainTimeout = 150 * time.Millisecond

func SetVersion(v string) {
	version = v
}

var (
	helpHeaderColor = color.New(color.Bold, color.FgCyan)
	helpCmdColor    = color.New(color.FgHiGreen)
	helpAliasColor  = color.New(color.FgYellow)
	helpHintColor   = color.New(color.Faint)
)

func helpHeader(s string) string { return helpHeaderColor.Sprint(s) }
func helpCmdCol(s string) string { return helpCmdColor.Sprint(s) }
func helpHint(s string) string   { return helpHintColor.Sprint(s) }
func helpAliases(aliases []string) string {
	return helpAliasColor.Sprintf("(aliases: %s)", strings.Join(aliases, ", "))
}

var groupOrder = []string{
	"Navigate:",
	"Observe:",
	"Input:",
	"Resources:",
	"Setup:",
}

func groupedHelp(cmd *cobra.Command) string {
	groups := map[string][]*cobra.Command{}
	for _, c := range cmd.Commands() {
		if !c.IsAvailableCommand() && c.Name() != "help" {
			continue
		}
		g := c.Annotations["group"]
		if g == "" {
			g = "Setup:"
		}
		groups[g] = append(groups[g], c)
	}

	var b strings.Builder
	for _, name := range groupOrder {
		cmds, ok := groups[name]
		if !ok {
			continue
		}
		b.WriteString("\n" + helpHeader(name) + "\n")
		for _, c := range cmds {
			line := "  " + helpCmdCol(fmt.Sprintf("%-14s", c.Name())) + " " + c.Short
			if len(c.Aliases) > 0 {
				line += " " + helpAliases(c.Aliases)
			}
			b.WriteString(line + "\n")
		}
	}
	return b.String()
}

const usageTemplate = `{{helpHeader "Usage:"}}{{if .Runnable}}
  {{.UseLine}}{{end}}{{if .HasAvailableSubCommands}}
  {{.CommandPath}} [command]{{end}}{{if gt (len .Aliases) 0}}

{{helpHeader "Aliases:"}}
  {{.NameAndAliases}}{{end}}{{if .HasExample}}

{{helpHeader "Examples:"}}
{{.Example}}{{end}}{{if .HasAvailableSubCommands}}
{{groupedHelp .}}{{end}}{{if .HasAvailableLocalFlags}}

{{helpHeader "Flags:"}}
{{.LocalFlags.FlagUsages | trimTrailingWhitespaces}}{{end}}{{if .HasAvailableInheritedFlags}}

{{helpHeader "Global Flags:"}}
{{.InheritedFlags.FlagUsages | trimTrailingWhitespaces}}{{end}}{{if .HasAvailableSubCommands}}

{{helpHint (printf "Use \"%s [command] --help\" for more information." .CommandPath)}}{{end}}
`

var rootCmd = &cobra.Command{
	Use:           "browseros-cli",
	Short:         "Browser control CLI for BrowserOS",
	Long:          "browseros-cli — command-line interface for controlling BrowserOS via MCP",
	SilenceUsage:  true,
	SilenceErrors: true,
}

func Execute() {
	automaticUpdater := newAutomaticUpdateManager(os.Args[1:])
	automaticNotice := ""
	var automaticCheckDone <-chan struct{}
	if automaticUpdater != nil {
		automaticNotice = automaticUpdater.CachedNotice()
		automaticCheckDone = automaticUpdater.StartBackgroundCheck(context.Background())
	}

	analytics.Init(version)
	start := time.Now()

	err := rootCmd.Execute()

	if automaticNotice != "" && err == nil {
		fmt.Fprintln(os.Stderr, automaticNotice)
	}
	drainAutomaticUpdateCheck(automaticCheckDone)

	analytics.Track(commandName(os.Args[1:]), err == nil, time.Since(start))
	analytics.Close()

	if err != nil {
		os.Exit(1)
	}
}

func commandName(args []string) string {
	cmd, _, err := rootCmd.Find(args)
	if err != nil || cmd == rootCmd {
		return "unknown"
	}
	return cmd.CommandPath()
}

func init() {
	cobra.AddTemplateFunc("helpHeader", helpHeader)
	cobra.AddTemplateFunc("helpCmdCol", helpCmdCol)
	cobra.AddTemplateFunc("helpAliases", helpAliases)
	cobra.AddTemplateFunc("helpHint", helpHint)
	cobra.AddTemplateFunc("groupedHelp", groupedHelp)

	rootCmd.SetUsageTemplate(usageTemplate)

	rootCmd.PersistentFlags().StringVarP(&serverURL, "server", "s", defaultServerURL(), "BrowserOS server URL")
	rootCmd.PersistentFlags().IntVarP(&pageFlag, "page", "p", 0, "Target page ID (default: active page)")
	rootCmd.PersistentFlags().BoolVar(&jsonOut, "json", envBool("BOS_JSON"), "JSON output")
	rootCmd.PersistentFlags().BoolVar(&debug, "debug", envBool("BOS_DEBUG"), "Debug output")
	rootCmd.PersistentFlags().DurationVarP(&timeout, "timeout", "t", 120*time.Second, "Request timeout")

	rootCmd.Version = version
}

func newClient() *mcp.Client {
	baseURL, err := validateServerURL(serverURL)
	if err != nil {
		output.Error(err.Error(), 1)
	}

	c := mcp.NewClient(baseURL, version, timeout)
	c.Debug = debug
	return c
}

func resolvePageID(c *mcp.Client) (int, error) {
	if rootCmd.PersistentFlags().Changed("page") {
		return pageFlag, nil
	}

	if env := os.Getenv("BROWSEROS_PAGE"); env != "" {
		if v, err := strconv.Atoi(env); err == nil {
			return v, nil
		}
	}

	return c.ResolvePageID(nil)
}

func envBool(key string) bool {
	v := os.Getenv(key)
	return v == "1" || v == "true"
}

func newAutomaticUpdateManager(args []string) *update.Manager {
	if shouldSkipAutomaticUpdates(args) {
		return nil
	}

	return update.NewManager(update.Options{
		CurrentVersion: version,
		JSONOutput:     requestedBoolFlag(args, "--json", jsonOut),
		Debug:          requestedBoolFlag(args, "--debug", debug),
		Automatic:      true,
	})
}

func shouldSkipAutomaticUpdates(args []string) bool {
	if hasHelpFlag(args) || requestedBoolFlag(args, "--version", false) {
		return true
	}

	switch primaryCommand(args) {
	case "help", "completion", "update", "self-update", "upgrade":
		return true
	default:
		return false
	}
}

func hasHelpFlag(args []string) bool {
	if requestedBoolFlag(args, "--help", false) {
		return true
	}

	for _, arg := range args {
		if arg == "-h" {
			return true
		}
	}

	return false
}

func primaryCommand(args []string) string {
	for _, arg := range args {
		if strings.HasPrefix(arg, "-") {
			continue
		}
		return arg
	}
	return ""
}

func requestedBoolFlag(args []string, flagName string, current bool) bool {
	if current {
		return true
	}

	prefix := flagName + "="
	for _, arg := range args {
		if arg == flagName {
			return true
		}
		if strings.HasPrefix(arg, prefix) {
			value, err := strconv.ParseBool(strings.TrimPrefix(arg, prefix))
			return err == nil && value
		}
	}

	return false
}

func drainAutomaticUpdateCheck(done <-chan struct{}) {
	drainAutomaticUpdateCheckWithTimeout(done, automaticUpdateDrainTimeout)
}

func drainAutomaticUpdateCheckWithTimeout(done <-chan struct{}, timeout time.Duration) {
	if done == nil {
		return
	}

	timer := time.NewTimer(timeout)
	defer timer.Stop()

	select {
	case <-done:
	case <-timer.C:
	}
}

func defaultServerURL() string {
	// 1. Explicit env var always wins
	if env := normalizeServerURL(os.Getenv("BROWSEROS_URL")); env != "" {
		return env
	}

	// 2. Live discovery file from running BrowserOS (most current)
	if url := loadBrowserosServerURL(); url != "" {
		return url
	}

	// 3. Saved config (may be stale if port changed)
	cfg, err := config.Load()
	if err == nil {
		if url := normalizeServerURL(cfg.ServerURL); url != "" {
			return url
		}
	}

	return ""
}

type serverDiscoveryConfig struct {
	ServerPort       int    `json:"server_port"`
	URL              string `json:"url"`
	ServerVersion    string `json:"server_version"`
	BrowserOSVersion string `json:"browseros_version,omitempty"`
	ChromiumVersion  string `json:"chromium_version,omitempty"`
}

func loadBrowserosServerURL() string {
	home, err := os.UserHomeDir()
	if err != nil {
		return ""
	}

	data, err := os.ReadFile(filepath.Join(home, ".browseros", "server.json"))
	if err != nil {
		return ""
	}

	var sc serverDiscoveryConfig
	if err := json.Unmarshal(data, &sc); err != nil {
		return ""
	}

	return normalizeServerURL(sc.URL)
}

func normalizeServerURL(raw string) string {
	normalized := strings.TrimSpace(raw)
	normalized = strings.TrimSuffix(normalized, "/mcp")
	return strings.TrimSuffix(normalized, "/")
}

func validateServerURL(raw string) (string, error) {
	baseURL := normalizeServerURL(raw)
	if baseURL != "" {
		return baseURL, nil
	}

	return "", fmt.Errorf(
		"BrowserOS server URL is not configured.\n\n" +
			"  If BrowserOS is running:  browseros-cli init --auto\n" +
			"  If BrowserOS is closed:   browseros-cli launch\n" +
			"  If not installed:         browseros-cli install",
	)
}
