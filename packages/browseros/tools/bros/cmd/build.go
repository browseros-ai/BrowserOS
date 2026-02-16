package cmd

import (
	nativebuild "bros/internal/native/build"

	"github.com/spf13/cobra"
)

type buildOptions struct {
	Config      string
	Modules     string
	List        bool
	Setup       bool
	Prep        bool
	Build       bool
	Sign        bool
	Package     bool
	Upload      bool
	Arch        string
	BuildType   string
	ChromiumSrc string
}

var (
	buildRootOpts buildOptions
	buildRunOpts  buildOptions
)

var buildCmd = &cobra.Command{
	Use:   "build",
	Short: "Build BrowserOS browser",
	Long:  "Build and patch-development commands for BrowserOS.",
	RunE: func(cmd *cobra.Command, args []string) error {
		return runBuildCommand(buildRootOpts)
	},
}

var buildRunCmd = &cobra.Command{
	Use:   "run",
	Short: "Run build pipeline",
	RunE: func(cmd *cobra.Command, args []string) error {
		return runBuildCommand(buildRunOpts)
	},
}

var buildModulesCmd = &cobra.Command{
	Use:   "modules",
	Short: "Module inspection commands",
	Run: func(cmd *cobra.Command, args []string) {
		_ = cmd.Help()
	},
}

var buildModulesListCmd = &cobra.Command{
	Use:   "list",
	Short: "List available build modules",
	RunE: func(cmd *cobra.Command, args []string) error {
		nativebuild.PrintModuleList()
		return nil
	},
}

func init() {
	addBuildFlags(buildCmd, &buildRootOpts)
	addBuildFlags(buildRunCmd, &buildRunOpts)

	buildModulesCmd.AddCommand(buildModulesListCmd)
	buildCmd.AddCommand(buildRunCmd, buildModulesCmd, newDevSurfaceCommand("patch", "Patch management commands"))
	rootCmd.AddCommand(buildCmd)
}

func addBuildFlags(cmd *cobra.Command, opts *buildOptions) {
	cmd.Flags().StringVarP(&opts.Config, "config", "c", "", "load configuration from YAML file")
	cmd.Flags().StringVarP(&opts.Modules, "modules", "m", "", "comma-separated list of modules to run")
	cmd.Flags().BoolVarP(&opts.List, "list", "l", false, "list all available modules and exit")
	cmd.Flags().BoolVar(&opts.Setup, "setup", false, "run setup phase")
	cmd.Flags().BoolVar(&opts.Prep, "prep", false, "run prep phase")
	cmd.Flags().BoolVar(&opts.Build, "build", false, "run build phase")
	cmd.Flags().BoolVar(&opts.Sign, "sign", false, "run sign phase")
	cmd.Flags().BoolVar(&opts.Package, "package", false, "run package phase")
	cmd.Flags().BoolVar(&opts.Upload, "upload", false, "run upload phase")
	cmd.Flags().StringVarP(&opts.Arch, "arch", "a", "", "target architecture")
	cmd.Flags().StringVarP(&opts.BuildType, "build-type", "t", "", "build type (debug|release)")
	cmd.Flags().StringVarP(&opts.ChromiumSrc, "chromium-src", "S", "", "path to Chromium source directory")
}

func runBuildCommand(opts buildOptions) error {
	return nativebuild.Run(nativebuild.Options{
		ConfigPath:  opts.Config,
		Modules:     opts.Modules,
		ListModules: opts.List,
		Setup:       opts.Setup,
		Prep:        opts.Prep,
		Build:       opts.Build,
		Sign:        opts.Sign,
		Package:     opts.Package,
		Upload:      opts.Upload,
		Arch:        opts.Arch,
		BuildType:   opts.BuildType,
		ChromiumSrc: opts.ChromiumSrc,
	})
}
