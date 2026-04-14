package cmd

import (
	"browseros-cli/output"

	"github.com/spf13/cobra"
)

func init() {
	strataCmd := &cobra.Command{
		Use:         "strata",
		Annotations: map[string]string{"group": "Integrations:"},
		Short:       "Manage Strata MCP integrations (Gmail, Slack, GitHub, etc.)",
		Long: `Interact with 40+ external services via Strata MCP integrations.

Supported services:
  gmail, google calendar, google docs, google drive, google sheets, slack,
  linkedin, notion, airtable, confluence, github, gitlab, linear, jira,
  figma, salesforce, hubspot, stripe, discord, asana, clickup, zendesk,
  monday, shopify, dropbox, onedrive, box, youtube, whatsapp, resend,
  posthog, mixpanel, vercel, supabase, cloudflare, wordpress, postman,
  intercom, cal.com, brave search, microsoft teams, outlook mail,
  outlook calendar, google forms, mem0

Discovery flow — do not guess action names:
  1. check     → verify the service is connected (get auth URL if not)
  2. discover  → find categories or actions for a service
  3. actions   → expand categories into specific actions
  4. details   → get the parameter schema before executing
  5. exec      → execute the action with parameters
  6. search    → fallback keyword search if discover doesn't find it

Authentication:
  If a service is not connected, "check" returns an authUrl.
  Open that URL in a browser to authenticate, then retry.
  If "exec" fails with an auth error, use "auth" to get a fresh authUrl.

Example — search Gmail:
  browseros-cli strata check gmail
  browseros-cli strata discover "search emails" gmail
  browseros-cli strata actions GMAIL_EMAIL
  browseros-cli strata details GMAIL_EMAIL gmail_search_emails
  browseros-cli strata exec gmail GMAIL_EMAIL gmail_search_emails \
    --body '{"query":"from:user@example.com","maxResults":5}'`,
	}

	checkCmd := &cobra.Command{
		Use:   "check <server-name>",
		Short: "Check if a service is connected and ready",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("connector_mcp_servers", map[string]any{
				"server_name": args[0],
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

	discoverCmd := &cobra.Command{
		Use:   "discover <query> <server> [servers...]",
		Short: "Discover available categories or actions for servers",
		Args:  cobra.MinimumNArgs(2),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("discover_server_categories_or_actions", map[string]any{
				"user_query":   args[0],
				"server_names": args[1:],
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

	actionsCmd := &cobra.Command{
		Use:   "actions <category> [categories...]",
		Short: "Get actions within categories",
		Args:  cobra.MinimumNArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("get_category_actions", map[string]any{
				"category_names": args,
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

	detailsCmd := &cobra.Command{
		Use:   "details <category> <action>",
		Short: "Get parameter schema for an action",
		Args:  cobra.ExactArgs(2),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("get_action_details", map[string]any{
				"category_name": args[0],
				"action_name":   args[1],
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

	execCmd := &cobra.Command{
		Use:   "exec <server> <category> <action>",
		Short: "Execute an action on a connected service",
		Long: `Execute an action on a connected service.

Pass request body as a JSON string with --body.
Use --query and --path for query/path parameters.
Use --output-field to limit response fields.

Example:
  browseros-cli strata exec gmail GMAIL_EMAIL gmail_search_emails \
    --body '{"query":"from:user@example.com","maxResults":5}'`,
		Args: cobra.ExactArgs(3),
		Run: func(cmd *cobra.Command, args []string) {
			bodySchema, _ := cmd.Flags().GetString("body")
			queryParams, _ := cmd.Flags().GetString("query")
			pathParams, _ := cmd.Flags().GetString("path")
			outputFields, _ := cmd.Flags().GetStringArray("output-field")
			maxChars, _ := cmd.Flags().GetInt("max-chars")

			toolArgs := map[string]any{
				"server_name":   args[0],
				"category_name": args[1],
				"action_name":   args[2],
			}

			if bodySchema != "" {
				toolArgs["body_schema"] = bodySchema
			}
			if queryParams != "" {
				toolArgs["query_params"] = queryParams
			}
			if pathParams != "" {
				toolArgs["path_params"] = pathParams
			}
			if len(outputFields) > 0 {
				toolArgs["include_output_fields"] = outputFields
			}
			if cmd.Flags().Changed("max-chars") {
				toolArgs["maximum_output_characters"] = maxChars
			}

			c := newClient()
			result, err := c.CallTool("execute_action", toolArgs)
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
	execCmd.Flags().String("body", "", "Request body as JSON string")
	execCmd.Flags().String("query", "", "Query parameters as JSON string")
	execCmd.Flags().String("path", "", "Path parameters as JSON string")
	execCmd.Flags().StringArray("output-field", nil, "Limit response to these fields (repeatable)")
	execCmd.Flags().Int("max-chars", 0, "Maximum output characters")

	searchCmd := &cobra.Command{
		Use:   "search <query> <server>",
		Short: "Search documentation for a service",
		Args:  cobra.ExactArgs(2),
		Run: func(cmd *cobra.Command, args []string) {
			c := newClient()
			result, err := c.CallTool("search_documentation", map[string]any{
				"query":       args[0],
				"server_name": args[1],
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

	authCmd := &cobra.Command{
		Use:   "auth <server-name>",
		Short: "Handle authentication failure for a service",
		Args:  cobra.ExactArgs(1),
		Run: func(cmd *cobra.Command, args []string) {
			intention, _ := cmd.Flags().GetString("intention")
			c := newClient()
			result, err := c.CallTool("handle_auth_failure", map[string]any{
				"server_name": args[0],
				"intention":   intention,
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
	authCmd.Flags().String("intention", "get_auth_url", "Auth intention")

	strataCmd.AddCommand(checkCmd, discoverCmd, actionsCmd, detailsCmd, execCmd, searchCmd, authCmd)
	rootCmd.AddCommand(strataCmd)
}
