package raw

import (
	"errors"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

// Register wires raw-tier commands into the root command.
func Register(root *cobra.Command, deps Deps) {
	root.AddCommand(newCDPCommand(deps))
}

func newCDPCommand(deps Deps) *cobra.Command {
	return &cobra.Command{
		Use:         "cdp <Domain.method> <json-params>",
		Short:       "Call one raw CDP method on a page",
		Args:        cobra.ExactArgs(2),
		Annotations: map[string]string{"group": "Raw:"},
		Example: `  browseros-cli -p 7 cdp Runtime.evaluate '{"expression":"document.title","returnByValue":true}'
  browseros-cli -p 7 --json cdp Page.getNavigationHistory '{}'`,
		Run: func(cmd *cobra.Command, args []string) {
			result, err := executeCDP(deps, args[0], args[1])
			if err != nil {
				if result != nil && deps.JSONOutput != nil && deps.JSONOutput() {
					output.JSON(result)
				}
				output.Error(err.Error(), errorCode(err))
			}
			if deps.JSONOutput != nil && deps.JSONOutput() {
				output.JSON(result)
			} else {
				output.Text(result)
			}
		},
	}
}

func errorCode(err error) int {
	var coded codedError
	if errors.As(err, &coded) {
		return coded.Code()
	}
	return 1
}
