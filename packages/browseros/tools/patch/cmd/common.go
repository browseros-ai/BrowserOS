package cmd

import (
	"fmt"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/engine"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/repo"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/ui"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/patch/internal/workspace"
	"github.com/spf13/cobra"
)

const srcFlagUsage = "Chromium checkout path to operate on directly without registry lookup"

func repoInfo() (*repo.Info, error) {
	return appState.RepoInfo()
}

func resolveWorkspace(cmd *cobra.Command, positional []string, src string) (workspace.Entry, error) {
	name := ""
	if len(positional) > 0 {
		name = positional[0]
	}
	commandPath := ""
	if cmd != nil {
		commandPath = cmd.CommandPath()
	}
	return workspace.ResolveForCommand(appState.Registry, name, appState.CWD, src, commandPath)
}

func splitWorkspaceAndFilters(cmd *cobra.Command, args []string) ([]string, []string) {
	atDash := cmd.ArgsLenAtDash()
	if atDash == -1 {
		return args, nil
	}
	return args[:atDash], args[atDash:]
}

// llmTxtGuide returns a stable plain-text operating guide for coding agents.
func llmTxtGuide() string {
	return `browseros-patch quick reference for coding agents

Terms:
- patch repo: BrowserOS packages/browseros repo containing chromium_patches/.
- Chromium checkout: local Chromium src tree registered with a checkout name like ch1.
- checkout name: registry alias used by commands, for example ch1.
- --src: operate on a Chromium checkout path directly without registry lookup.

Rules:
- Checkout commands work from anywhere when passed a checkout name: browseros-patch diff ch1.
- browseros-patch list reads only registered Chromium checkouts; it does not inspect sync state.
- Use browseros-patch status ch1 or browseros-patch diff ch1 before mutating.
- Mutating commands: browseros-patch sync ch1, browseros-patch apply ch1, browseros-patch extract ch1.
`
}

func ensureRepoConfigured(override string) error {
	if override == "" && appState.Config.PatchesRepo != "" {
		return nil
	}
	root := override
	if root == "" {
		discovered, err := repo.Discover(appState.CWD)
		if err != nil {
			return fmt.Errorf(`unable to discover patches repo; pass --patches-repo or run from packages/browseros`)
		}
		root = discovered
	}
	info, err := repo.Load(root)
	if err != nil {
		return err
	}
	appState.Config.PatchesRepo = info.Root
	return nil
}

// commandProgress routes long-running engine updates to stderr in human mode only.
func commandProgress(cmd *cobra.Command) engine.Progress {
	if jsonOut {
		return nil
	}
	return engine.ProgressFunc(func(message string) {
		fmt.Fprintf(cmd.ErrOrStderr(), "%s %s\n", ui.Muted("..."), message)
	})
}
