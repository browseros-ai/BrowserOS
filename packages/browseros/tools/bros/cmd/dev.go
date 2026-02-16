package cmd

import (
	nativedev "bros/internal/native/dev"

	"github.com/spf13/cobra"
)

type devOptions struct {
	ChromiumSrc string
	Verbose     bool
	Quiet       bool
}

func init() {
	rootCmd.AddCommand(newDevSurfaceCommand("dev", "Patch development commands"))
}

func newDevSurfaceCommand(use string, short string) *cobra.Command {
	opts := &devOptions{}

	devCmd := &cobra.Command{
		Use:   use,
		Short: short,
	}

	devCmd.PersistentFlags().StringVarP(&opts.ChromiumSrc, "chromium-src", "S", "", "path to Chromium source directory")
	devCmd.PersistentFlags().BoolVarP(&opts.Verbose, "verbose", "v", false, "enable verbose output")
	devCmd.PersistentFlags().BoolVarP(&opts.Quiet, "quiet", "q", false, "suppress non-essential output")

	statusCmd := &cobra.Command{
		Use:   "status",
		Short: "Show patch dev status",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runDevCommand(opts, []string{"status"}, nil)
		},
	}

	annotateCmd := &cobra.Command{
		Use:   "annotate [feature_name]",
		Short: "Create git commits organized by features",
		Args:  cobra.MaximumNArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			return runDevCommand(opts, []string{"annotate"}, args)
		},
	}

	featureCmd := &cobra.Command{
		Use:   "feature",
		Short: "Feature management commands",
		Run: func(cmd *cobra.Command, args []string) {
			_ = cmd.Help()
		},
	}

	featureListCmd := &cobra.Command{
		Use:   "list",
		Short: "List all defined features",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runDevCommand(opts, []string{"feature", "list"}, nil)
		},
	}

	featureShowCmd := &cobra.Command{
		Use:   "show <feature_name>",
		Short: "Show details for a feature",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			return runDevCommand(opts, []string{"feature", "show"}, args)
		},
	}

	featureClassifyCmd := &cobra.Command{
		Use:   "classify",
		Short: "Classify unassigned patch files into features",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			return runDevCommand(opts, []string{"feature", "classify"}, nil)
		},
	}

	var featureName string
	var featureCommit string
	var featureDescription string
	featureAddUpdateCmd := &cobra.Command{
		Use:   "add-update",
		Short: "Add or update a feature using commit files",
		Args:  cobra.NoArgs,
		RunE: func(cmd *cobra.Command, args []string) error {
			devArgs := []string{
				"--name", featureName,
				"--commit", featureCommit,
				"--description", featureDescription,
			}
			return runDevCommand(opts, []string{"feature", "add-update"}, devArgs)
		},
	}
	featureAddUpdateCmd.Flags().StringVarP(&featureName, "name", "n", "", "feature name (lowercase kebab-case)")
	featureAddUpdateCmd.Flags().StringVarP(&featureCommit, "commit", "c", "", "git commit reference")
	featureAddUpdateCmd.Flags().StringVarP(&featureDescription, "description", "d", "", "feature description with prefix (feat:, fix:, build:, chore:, series:)")
	_ = featureAddUpdateCmd.MarkFlagRequired("name")
	_ = featureAddUpdateCmd.MarkFlagRequired("commit")
	_ = featureAddUpdateCmd.MarkFlagRequired("description")

	featureCmd.AddCommand(featureListCmd, featureShowCmd, featureAddUpdateCmd, featureClassifyCmd)
	devCmd.AddCommand(statusCmd, annotateCmd, featureCmd)

	return devCmd
}

func runDevCommand(opts *devOptions, trail []string, passthrough []string) error {
	base := []string{}

	if opts.ChromiumSrc != "" {
		base = append(base, "--chromium-src", opts.ChromiumSrc)
	}
	if opts.Verbose {
		base = append(base, "--verbose")
	}
	if opts.Quiet {
		base = append(base, "--quiet")
	}

	base = append(base, trail...)
	base = append(base, passthrough...)
	return nativedev.Run(base)
}
