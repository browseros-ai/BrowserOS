package cmd

import (
	"bufio"
	"context"
	"fmt"
	"io"
	"os"
	"strings"

	"browseros-cli/output"
	"browseros-cli/update"

	"github.com/spf13/cobra"
)

type updateManager interface {
	CheckNow(context.Context) (*update.CheckResult, error)
	Apply(context.Context, *update.CheckResult) error
}

type updateOutcome struct {
	result   *update.CheckResult
	applied  bool
	canceled bool
}

func init() {
	cmd := &cobra.Command{
		Use:         "update",
		Aliases:     []string{"self-update", "upgrade"},
		Annotations: map[string]string{"group": "Setup:"},
		Short:       "Check for and apply CLI updates",
		Args:        cobra.NoArgs,
		Run: func(cmd *cobra.Command, args []string) {
			checkOnly, _ := cmd.Flags().GetBool("check")
			yes, _ := cmd.Flags().GetBool("yes")

			manager := update.NewManager(update.Options{
				CurrentVersion: version,
				JSONOutput:     jsonOut,
				Debug:          debug,
				Automatic:      false,
			})
			outcome, err := runUpdateCommand(
				cmd.Context(),
				manager,
				checkOnly,
				yes,
				stdinIsInteractive(os.Stdin),
				os.Stdin,
				os.Stderr,
			)
			if err != nil {
				output.Error(err.Error(), 1)
			}
			printUpdateOutcome(outcome)
		},
	}

	cmd.Flags().Bool("check", false, "Check for updates without applying them")
	cmd.Flags().Bool("yes", false, "Apply update without prompting")

	rootCmd.AddCommand(cmd)
}

func runUpdateCommand(
	ctx context.Context,
	manager updateManager,
	checkOnly bool,
	yes bool,
	interactive bool,
	stdin io.Reader,
	stderr io.Writer,
) (*updateOutcome, error) {
	result, err := manager.CheckNow(ctx)
	if err != nil {
		return nil, err
	}

	outcome := &updateOutcome{result: result}
	if checkOnly || !result.UpdateAvailable {
		return outcome, nil
	}
	if !yes {
		if !interactive {
			return nil, fmt.Errorf("update requires confirmation; rerun with --yes")
		}

		confirmed, err := confirmUpdate(stdin, stderr, result)
		if err != nil {
			return nil, err
		}
		if !confirmed {
			outcome.canceled = true
			return outcome, nil
		}
	}
	if err := manager.Apply(ctx, result); err != nil {
		return nil, err
	}
	outcome.applied = true

	return outcome, nil
}

func printUpdateOutcome(outcome *updateOutcome) {
	if jsonOut {
		output.JSONRaw(updateOutcomePayload(outcome))
		return
	}

	switch {
	case outcome.applied:
		fmt.Printf("Updated browseros-cli to v%s\n", outcome.result.LatestVersion)
	case outcome.canceled:
		fmt.Println("Update canceled.")
	case outcome.result.UpdateAvailable:
		fmt.Println(update.FormatNotice(outcome.result.CurrentVersion, outcome.result.LatestVersion))
	case outcome.result != nil:
		fmt.Printf("browseros-cli is up to date (v%s)\n", outcome.result.CurrentVersion)
	}
}

func updateOutcomePayload(outcome *updateOutcome) map[string]any {
	payload := map[string]any{
		"applied": outcome.applied,
	}
	if outcome.canceled {
		payload["canceled"] = true
	}
	if outcome.result == nil {
		return payload
	}

	payload["currentVersion"] = outcome.result.CurrentVersion
	payload["latestVersion"] = outcome.result.LatestVersion
	payload["updateAvailable"] = outcome.result.UpdateAvailable
	if outcome.result.Asset != nil {
		payload["asset"] = map[string]any{
			"filename":      outcome.result.Asset.Filename,
			"url":           outcome.result.Asset.URL,
			"archiveFormat": outcome.result.Asset.ArchiveFormat,
		}
	}

	return payload
}

func confirmUpdate(
	stdin io.Reader,
	stderr io.Writer,
	result *update.CheckResult,
) (bool, error) {
	if _, err := fmt.Fprintf(
		stderr,
		"Install browseros-cli v%s over v%s? [y/N]: ",
		result.LatestVersion,
		result.CurrentVersion,
	); err != nil {
		return false, err
	}

	line, err := bufio.NewReader(stdin).ReadString('\n')
	if err != nil && err != io.EOF {
		return false, err
	}
	answer := strings.ToLower(strings.TrimSpace(line))

	return answer == "y" || answer == "yes", nil
}

func stdinIsInteractive(file *os.File) bool {
	info, err := file.Stat()
	if err != nil {
		return false
	}
	return info.Mode()&os.ModeCharDevice != 0
}
