package cmd

import (
	"bufio"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"browseros-dogfood/config"
	"browseros-dogfood/pipeline"
	"browseros-dogfood/profile"

	"github.com/spf13/cobra"
)

func init() {
	rootCmd.AddCommand(initCmd)
}

var initCmd = &cobra.Command{
	Use:     "init",
	Short:   "Create or update dogfood config",
	GroupID: groupSetup,
	RunE: func(cmd *cobra.Command, args []string) error {
		target, ok, err := selectedTarget()
		if err != nil {
			return err
		}
		if !ok {
			return fmt.Errorf("choose a dogfood target with --browseros or --claw")
		}
		home, err := os.UserHomeDir()
		if err != nil {
			return err
		}
		path, err := config.Path()
		if err != nil {
			return err
		}
		cfg, err := loadInitConfig(home, path, target)
		if err != nil {
			return err
		}
		if cwd, err := os.Getwd(); err == nil && cfg.RepoPath == "" && looksLikeRepo(cwd) {
			cfg.RepoPath = cwd
		}
		reader := bufio.NewReader(os.Stdin)
		out := cmd.OutOrStdout()
		printRepoPathHelp(out)
		cfg.RepoPath = prompt(out, reader, "Repo path", cfg.RepoPath)
		if branch := pipeline.Branch(cfg.RepoPath, pipeline.ExecRunner{}); branch != "" {
			cfg.Branch = branch
		}
		printBranchHelp(out)
		cfg.Branch = promptValue(out, reader, "Branch", cfg.Branch)
		cfg.BrowserOSAppPath = prompt(out, reader, "BrowserOS binary", cfg.BrowserOSAppPath)
		profiles, _ := profile.ReadProfiles(cfg.SourceUserDataDir)
		if len(profiles) > 0 {
			printSourceProfileHelp(out)
		}
		cfg.SourceProfileDir = chooseProfile(out, reader, profiles)
		cfg.Resolve()
		if err := cfg.Validate(); err != nil {
			return err
		}
		if err := config.Save(path, cfg); err != nil {
			return err
		}
		if err := pipeline.WriteProductionEnvFile(cfg.AgentRoot(), cfg); err != nil {
			return err
		}
		printInitNextSteps(cmd.OutOrStdout(), path, target)
		return nil
	},
}

// loadInitConfig preserves existing target settings while allowing first-run defaults.
func loadInitConfig(home string, path string, target config.Target) (config.Config, error) {
	cfg := config.Defaults(home)
	if existing, err := config.Load(path); err == nil {
		cfg = existing
	} else if !os.IsNotExist(err) {
		return config.Config{}, fmt.Errorf("load existing config: %w", err)
	}
	if err := cfg.ApplyTarget(target); err != nil {
		return config.Config{}, err
	}
	return cfg, nil
}

func printInitNextSteps(out io.Writer, path string, target config.Target) {
	targetFlag, _ := selectedTargetFlag(target)
	fmt.Fprintf(out, "%s %s\n", successStyle.Sprint("Config written:"), pathStyle.Sprint(path))
	fmt.Fprintf(out, "%s %s\n", labelStyle.Sprint("Start dogfood:"), targetLabel(target))
	fmt.Fprintf(out, "  %s     %s\n", labelStyle.Sprint("Inline:"), commandStyle.Sprintf("browseros-dogfood %s start", targetFlag))
	fmt.Fprintf(out, "  %s %s\n", labelStyle.Sprint("Background:"), commandStyle.Sprintf("browseros-dogfood %s start-background", targetFlag))
}

func printRepoPathHelp(out io.Writer) {
	fmt.Fprintln(out, "Repo path is the root BrowserOS repo clone for alpha dogfood.")
	fmt.Fprintln(out, "Use a separate clone from your everyday dev checkout if you can.")
	fmt.Fprintln(out, "Example: /Users/you/code/browseros-alpha, not packages/browseros-agent.")
}

func printBranchHelp(out io.Writer) {
	fmt.Fprintln(out, "Branch is the BrowserOS branch dogfood should track during pull/restart --pull.")
}

func printSourceProfileHelp(out io.Writer) {
	fmt.Fprintln(out, "Choose the installed BrowserOS profile you normally use.")
	fmt.Fprintln(out, "  Dogfood copies it into a separate dev profile.")
}

func prompt(out io.Writer, r *bufio.Reader, label string, current string) string {
	fmt.Fprintf(out, "%s [%s]: ", labelStyle.Sprint(label), pathStyle.Sprint(current))
	line, _ := r.ReadString('\n')
	line = strings.TrimSpace(line)
	if line == "" {
		return current
	}
	home, _ := os.UserHomeDir()
	return config.ExpandTilde(line, home)
}

func promptValue(out io.Writer, r *bufio.Reader, label string, current string) string {
	fmt.Fprintf(out, "%s [%s]: ", labelStyle.Sprint(label), commandStyle.Sprint(current))
	line, _ := r.ReadString('\n')
	line = strings.TrimSpace(line)
	if line == "" {
		return current
	}
	return line
}

func chooseProfile(out io.Writer, r *bufio.Reader, profiles []profile.BrowserProfile) string {
	if len(profiles) == 0 {
		return "Default"
	}
	fmt.Fprintf(out, "%s %d BrowserOS profiles:\n", labelStyle.Sprint("Found"), len(profiles))
	for i, p := range profiles {
		email := ""
		if p.Email != "" {
			email = " " + p.Email
		}
		fmt.Fprintf(out, "  %s %s (%s)%s\n", commandStyle.Sprintf("%d.", i+1), p.Name, pathStyle.Sprint(p.Dir), email)
	}
	for {
		fmt.Fprintf(out, "%s [1]: ", labelStyle.Sprint("Select source profile"))
		line, _ := r.ReadString('\n')
		line = strings.TrimSpace(line)
		if line == "" {
			return profiles[0].Dir
		}
		n, err := strconv.Atoi(line)
		if err == nil && n >= 1 && n <= len(profiles) {
			return profiles[n-1].Dir
		}
		fmt.Fprintln(out, warnStyle.Sprint("Choose a listed number."))
	}
}

func looksLikeRepo(path string) bool {
	_, err := os.Stat(filepath.Join(path, "packages/browseros-agent/package.json"))
	return err == nil
}
