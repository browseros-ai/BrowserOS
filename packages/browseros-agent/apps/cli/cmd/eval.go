package cmd

import (
	"fmt"
	"strings"

	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	cmd := &cobra.Command{
		Use:         "eval <expression>",
		Annotations: map[string]string{"group": "Observe:"},
		Short:       "Execute JavaScript in the page context",
		Args:        cobra.MinimumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			expression := strings.Join(args, " ")
			pageID, err := resolvePageID(nil)
			if err != nil {
				output.Error(err.Error(), 2)
			}
			c := newClient()
			result, err := c.CallTool("evaluate", map[string]any{
				"page": pageID,
				"code": evalCode(expression),
			})
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

	rootCmd.AddCommand(cmd)
}

// evalCode preserves the old expression-style CLI while still accepting statement snippets.
func evalCode(source string) string {
	return fmt.Sprintf(`const source = %s
const isSyntaxError = (err) => err instanceof SyntaxError || err?.name === 'SyntaxError'
try {
  return await (0, eval)(`+"`(async () => (${source}))()`"+`)
} catch (expressionError) {
  if (!isSyntaxError(expressionError)) throw expressionError
  try {
    const value = (0, eval)(source)
    return value && typeof value.then === 'function' ? await value : value
  } catch (statementError) {
    if (!isSyntaxError(statementError)) throw statementError
    return await (0, eval)(`+"`(async () => { ${source} })()`"+`)
  }
}`, jsLiteral(source))
}
