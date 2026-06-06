/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS MCP Bridge — MCP Server
 * Exposes high-level vision + git workflow tools that combine BrowserOS and GitButler.
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { z } from 'zod'
import { absorbSmart } from './absorb/absorb-engine.js'
import type { StrategyName } from './absorb/strategies.js'
import type { BrowserOSClient } from './clients/browseros-client'
import type { GitButlerMcpClient } from './clients/gitbutler-client'
import type { GitHubMcpClient } from './clients/github-client'
import type { RailwayMcpClient } from './clients/railway-client'
import type { TriClient } from './clients/tri-client'
import type { TriosRagClient } from './clients/trios-rag-client'
import type { BridgeConfig } from './config'
import type { FileChange } from './types'
import {
  type AnalyzedUI,
  analyzeGitButlerUI,
  type RawStatus,
} from './vision/gitbutler-analyzer.js'

export interface BridgeDeps {
  config: BridgeConfig
  browseros: BrowserOSClient
  gitbutler: GitButlerMcpClient
  tri: TriClient
  rag: TriosRagClient
  railway: RailwayMcpClient | null
  github: GitHubMcpClient | null
}

const BRIDGE_INSTRUCTIONS = `TRIOS MCP Bridge — Vision-enhanced GitButler workflows.

This bridge connects BrowserOS (browser vision/control) with GitButler (virtual branches, stacks, commits).

## Workflow Pattern: See → Understand → Act

1. **See**: Use gitbutler_analyze_ui to take a screenshot of GitButler and understand the current state.
2. **Understand**: Use gitbutler_workspace_status to get detailed file/branch information.
3. **Act**: Use gitbutler_commit_visible, gitbutler_create_branch, gitbutler_push_stack to perform actions.
4. **Research**: Use search_chapters, get_chapter, list_chapters to query the GOLDEN BRIDGE compendium via RAG.
5. **PhD Pipeline**: Use build_pdf (dry-run by default), build_cover, list_claims, get_honest_counters for PDF generation and claim auditing.
6. **Deploy**: Use railway_redeploy, railway_deploy, railway_list_services, railway_fleet_health to manage Railway deployments from chat.
7. **Code**: Use github_repo_info, github_read_file, github_list_issues, github_create_issue, github_create_pr, github_search_code to manage GitHub repos from chat.

## Key Concepts

- **Virtual Branches**: GitButler uses virtual branches (not git branches). Create with gitbutler_create_branch.
- **Stacks**: Branches can be stacked. Push with gitbutler_push_stack.
- **Absorb**: Smart commit — GitButler figures out which commit each change belongs to.
- **Vision**: Screenshots of GitButler UI give visual context that CLI alone cannot provide.
- **RAG**: trios-mcp-rag provides semantic access to 80+ Trinity S³AI chapters via PostgreSQL.

## Tips

- Always call gitbutler_analyze_ui first to understand the current state.
- Use gitbutler_workspace_status for detailed file-level information.
- Combine vision + status for the most accurate understanding.
- The bridge handles reconnection to both BrowserOS and GitButler automatically.`

export function createBridgeServer(deps: BridgeDeps): McpServer {
  const server = new McpServer(
    {
      name: 'trios-mcp-bridge',
      title: 'TRIOS MCP Bridge — Vision + GitButler',
      version: '0.1.0',
    },
    { capabilities: { logging: {} }, instructions: BRIDGE_INSTRUCTIONS },
  )

  // ==========================================
  // Tool 1: Analyze GitButler UI (Vision)
  // ==========================================
  server.tool(
    'gitbutler_analyze_ui',
    'Take a screenshot of the GitButler UI and analyze the current state. ' +
      'Returns structured JSON: activeBranch, changedFiles (with paths + staged status), stacks, isClean, suggestedActions. ' +
      'ALWAYS call this first before any git operation to understand the current workspace state.',
    {
      page_id: z
        .number()
        .optional()
        .describe(
          'Page ID of the GitButler tab. If not provided, auto-detects the GitButler tab.',
        ),
    },
    async (args) => {
      try {
        // Find GitButler page
        let pageId = args.page_id
        if (!pageId) {
          const page = await deps.browseros.findGitButlerPage()
          if (!page) {
            return {
              content: [
                {
                  type: 'text',
                  text: JSON.stringify({
                    error: 'GitButler tab not found',
                    activeBranch: null,
                    changedFiles: [],
                    stacks: [],
                    isClean: true,
                    summary:
                      'GitButler tab not found. Please open GitButler in the browser first.',
                    suggestedActions: ['Open GitButler in the browser'],
                  }),
                },
              ],
              isError: true,
            }
          }
          pageId = page.id
        }

        // Take screenshot + snapshot in parallel
        const [screenshot, snapshot] = await Promise.allSettled([
          deps.browseros.takeScreenshot(pageId),
          deps.browseros.takeSnapshot(pageId),
        ])

        // Get CLI status for ground truth
        let cliStatus: RawStatus | null = null
        try {
          const status = await deps.gitbutler.getStatus()
          cliStatus = {
            branch: status.branch,
            ahead: status.ahead,
            behind: status.behind,
            staged: status.staged.map((f: FileChange) => ({
              path: f.path,
              status: f.status,
              oldPath: f.oldPath,
            })),
            unstaged: status.unstaged.map((f: FileChange) => ({
              path: f.path,
              status: f.status,
              oldPath: f.oldPath,
            })),
            untracked: status.untracked,
            conflicted: status.conflicted,
          }
        } catch (err) {
          console.warn('[Bridge] CLI status unavailable:', err)
        }

        // Get snapshot text
        const snapshotText =
          snapshot.status === 'fulfilled' ? snapshot.value.snapshot : null

        // Run structured analysis
        const analysis: AnalyzedUI = analyzeGitButlerUI(snapshotText, cliStatus)

        // Build response with both structured JSON + human-readable text + screenshot
        const parts: any[] = []

        // Structured JSON output (primary — for programmatic consumption)
        const structuredOutput = {
          activeBranch: analysis.activeBranch,
          changedFiles: analysis.changedFiles.map((f) => ({
            path: f.path,
            status: f.status,
            staged: f.staged,
          })),
          stacks: analysis.stacks,
          isClean: analysis.isClean,
          summary: analysis.summary,
          suggestedActions: analysis.suggestedActions,
        }

        // Human-readable markdown
        let humanText = '## GitButler UI Analysis\n\n'
        humanText += `**Branch:** ${analysis.activeBranch}\n`
        humanText += `**Clean:** ${analysis.isClean ? 'Yes ✅' : 'No ❌'}\n`
        humanText += `**Summary:** ${analysis.summary}\n\n`

        if (analysis.changedFiles.length > 0) {
          humanText += `### Changed Files (${analysis.changedFiles.length})\n`
          for (const f of analysis.changedFiles) {
            const stageMarker = f.staged ? '✅ staged' : '⬜ unstaged'
            humanText += `- [${f.status}] ${f.path} — ${stageMarker}\n`
          }
        }

        if (analysis.stacks.length > 0) {
          humanText += `\n### Stacks\n`
          for (const s of analysis.stacks) {
            humanText += `- ${s}\n`
          }
        }

        if (analysis.suggestedActions.length > 0) {
          humanText += `\n### Suggested Actions\n`
          for (const a of analysis.suggestedActions) {
            humanText += `- ${a}\n`
          }
        }

        humanText += `\n### Structured JSON\n\`\`\`json\n${JSON.stringify(structuredOutput, null, 2)}\n\`\`\`\n`

        // Add screenshot if available
        if (screenshot.status === 'fulfilled') {
          parts.push({
            type: 'image' as const,
            data: screenshot.value.data,
            mimeType: screenshot.value.mimeType,
          })
        }

        parts.unshift({ type: 'text' as const, text: humanText })
        return { content: parts }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                error: `Analysis failed: ${error instanceof Error ? error.message : String(error)}`,
                activeBranch: null,
                changedFiles: [],
                stacks: [],
                isClean: true,
                summary: 'Analysis failed — see error field',
                suggestedActions: ['Retry the analysis'],
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 2: Workspace Status
  // ==========================================
  server.tool(
    'gitbutler_workspace_status',
    'Get detailed workspace status including branches, changed files, and stack information. ' +
      'Use after gitbutler_analyze_ui for detailed file-level data.',
    {
      include_branches: z
        .boolean()
        .default(true)
        .describe('Include branch listing'),
    },
    async (args) => {
      try {
        const status = await deps.gitbutler.getStatus()
        let result = `## Workspace Status\n\n`
        result += `- **Branch:** ${status.branch}\n`
        result += `- **Ahead:** ${status.ahead} | **Behind:** ${status.behind}\n`
        result += `- **Staged:** ${status.staged.length} | **Unstaged:** ${status.unstaged.length} | **Untracked:** ${status.untracked.length} | **Conflicts:** ${status.conflicted.length}\n`

        if (status.staged.length > 0) {
          result += `\n### Staged Changes\n`
          for (const f of status.staged) {
            result += `- \`${f.status}\` ${f.path}\n`
          }
        }

        if (status.unstaged.length > 0) {
          result += `\n### Unstaged Changes\n`
          for (const f of status.unstaged) {
            result += `- \`${f.status}\` ${f.path}\n`
          }
        }

        if (status.untracked.length > 0) {
          result += `\n### Untracked Files\n`
          for (const f of status.untracked) {
            result += `- ${f}\n`
          }
        }

        if (status.conflicted.length > 0) {
          result += `\n### ⚠️ Conflicts\n`
          for (const f of status.conflicted) {
            result += `- ${f}\n`
          }
        }

        if (args.include_branches) {
          try {
            const branches = await deps.gitbutler.getBranches()
            if (branches.length > 0) {
              result += `\n### Branches\n`
              for (const b of branches) {
                const marker = b.isCurrent ? ' **← current**' : ''
                result += `- ${b.name} (↑${b.ahead} ↓${b.behind})${marker}\n`
              }
            }
          } catch {
            result += `\n⚠️ Branch listing unavailable\n`
          }
        }

        return { content: [{ type: 'text', text: result }] }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: `Error getting workspace status: ${error instanceof Error ? error.message : String(error)}`,
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 3: Commit Visible Changes
  // ==========================================
  server.tool(
    'gitbutler_commit_visible',
    'Commit currently visible/changed files with a descriptive message. ' +
      "Uses GitButler's smart commit which auto-stages changed files. " +
      'Call gitbutler_analyze_ui first to see what will be committed.',
    {
      message: z
        .string()
        .min(1)
        .max(2048)
        .describe('Commit message describing the changes'),
      files: z
        .array(z.string())
        .optional()
        .describe(
          'Specific files to commit. If omitted, commits all changed files.',
        ),
    },
    async (args) => {
      try {
        // Stage specific files if provided
        if (args.files && args.files.length > 0) {
          await deps.gitbutler.stage(args.files)
        }

        const result = await deps.gitbutler.commit(args.message)

        if (result.success) {
          return {
            content: [
              {
                type: 'text',
                text: `✅ Committed successfully!\n- **Hash:** ${result.hash}\n- **Message:** ${args.message}${args.files ? `\n- **Files:** ${args.files.join(', ')}` : ''}`,
              },
            ],
          }
        }
        return {
          content: [
            {
              type: 'text',
              text: `❌ Commit failed: ${result.error}`,
            },
          ],
          isError: true,
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: `Error committing: ${error instanceof Error ? error.message : String(error)}`,
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 4: Create Virtual Branch
  // ==========================================
  server.tool(
    'gitbutler_create_branch',
    'Create a new GitButler virtual branch. Virtual branches allow stacking changes independently.',
    {
      name: z
        .string()
        .min(1)
        .describe(
          "Name for the new branch (e.g. 'feature/auth', 'fix/login-bug')",
        ),
      base: z
        .string()
        .optional()
        .describe('Base branch to create from. Defaults to current branch.'),
    },
    async (args) => {
      try {
        const result = await deps.gitbutler.createBranch(args.name, args.base)
        return {
          content: [
            {
              type: 'text',
              text: `✅ Branch created: **${args.name}**${args.base ? ` (based on ${args.base})` : ''}\n${result}`,
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: `Error creating branch: ${error instanceof Error ? error.message : String(error)}`,
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 5: Push Stack
  // ==========================================
  server.tool(
    'gitbutler_push_stack',
    "Push the current stack/branch to remote. Equivalent to 'but push' in GitButler.",
    {
      branch: z
        .string()
        .optional()
        .describe(
          'Specific branch to push. If omitted, pushes current branch.',
        ),
    },
    async (args) => {
      try {
        const result = await deps.gitbutler.push(args.branch)
        return {
          content: [
            {
              type: 'text',
              text: `✅ Pushed successfully!\n${result}`,
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: `Error pushing: ${error instanceof Error ? error.message : String(error)}`,
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 6: Stage Files
  // ==========================================
  server.tool(
    'gitbutler_stage',
    'Stage specific files for commit in GitButler. Use before gitbutler_commit_visible.',
    {
      files: z.array(z.string()).min(1).describe('List of file paths to stage'),
    },
    async (args) => {
      try {
        const result = await deps.gitbutler.stage(args.files)
        return {
          content: [
            {
              type: 'text',
              text: `✅ Staged ${args.files.length} file(s):\n${args.files.map((f) => `- ${f}`).join('\n')}\n${result}`,
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: `Error staging files: ${error instanceof Error ? error.message : String(error)}`,
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 7: Absorb Changes
  // ==========================================
  server.tool(
    'gitbutler_absorb',
    'Smart absorb: GitButler automatically figures out which commit each change belongs to ' +
      'and amends changes into the appropriate commits. Like interactive staging but automatic.',
    {},
    async () => {
      try {
        const result = await deps.gitbutler.absorb()
        return {
          content: [
            {
              type: 'text',
              text: `✅ Changes absorbed into appropriate commits!\n${result}`,
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: `Error absorbing changes: ${error instanceof Error ? error.message : String(error)}`,
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 7b: Absorb Smart (Issue #6)
  // ==========================================
  server.tool(
    'gitbutler_absorb_smart',
    'Smart file sorting into virtual branches. ' +
      'Analyzes changed files and groups them into separate branches using a chosen strategy. ' +
      'Use dryRun=true first to see the plan, then dryRun=false to execute.\n\n' +
      'Strategies:\n' +
      "- 'by-directory': Groups files by top-level directory (src/, docs/, tests/)\n" +
      "- 'by-type': Groups files by type (TypeScript, styles, docs, config)\n" +
      "- 'auto': Picks the best strategy automatically",
    {
      strategy: z
        .enum(['by-directory', 'by-type', 'auto'])
        .default('auto')
        .describe('Sorting strategy: by-directory, by-type, or auto'),
      dryRun: z
        .boolean()
        .default(true)
        .describe(
          'If true, only show the plan without executing. Set false to apply.',
        ),
    },
    async (args) => {
      try {
        const result = await absorbSmart(
          { gitbutler: deps.gitbutler },
          args.strategy as StrategyName,
          args.dryRun,
        )

        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify(result, null, 2),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 8: Undo Last Commit
  // ==========================================
  server.tool(
    'gitbutler_undo_last_commit',
    'Undo the last commit. Changes return to unstaged/staged state. ' +
      'Useful when you committed to the wrong branch or need to amend the message. ' +
      "The undone commit's files remain in the working directory.",
    {},
    async () => {
      try {
        const result = await deps.gitbutler.undoLastCommit()
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: true,
                reason: `Undone commit "${result.message}" (HEAD is now ${result.hash}). Files are back in staged state.`,
                undoneHash: result.hash,
                undoneMessage: result.message,
              }),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 9: Pull Latest
  // ==========================================
  server.tool(
    'gitbutler_pull',
    'Pull latest changes for all applied branches. Updates the workspace with remote changes.',
    {},
    async () => {
      try {
        const result = await deps.gitbutler.pull()
        return {
          content: [
            {
              type: 'text',
              text: `✅ Pulled latest changes!\n${result}`,
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: `Error pulling: ${error instanceof Error ? error.message : String(error)}`,
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 9: Take GitButler Screenshot
  // ==========================================
  server.tool(
    'gitbutler_screenshot',
    'Take a raw screenshot of the GitButler tab. Returns the image for visual inspection. ' +
      'Use gitbutler_analyze_ui for automatic analysis instead.',
    {
      full_page: z
        .boolean()
        .default(false)
        .describe('Capture the full scrollable page'),
    },
    async (_args) => {
      try {
        const page = await deps.browseros.findGitButlerPage()
        if (!page) {
          return {
            content: [
              {
                type: 'text',
                text: '❌ GitButler tab not found. Please open GitButler in the browser.',
              },
            ],
            isError: true,
          }
        }

        const screenshot = await deps.browseros.takeScreenshot(page.id)
        return {
          content: [
            {
              type: 'text',
              text: `Screenshot of GitButler (page ${page.id}, ${page.url})`,
            },
            {
              type: 'image',
              data: screenshot.data,
              mimeType: screenshot.mimeType,
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: `Error taking screenshot: ${error instanceof Error ? error.message : String(error)}`,
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 10: Bridge Health Check
  // ==========================================
  server.tool(
    'gitbutler_bridge_health',
    'Check the health of the TRIOS MCP Bridge and its connections to BrowserOS and GitButler.',
    {},
    async () => {
      const checks: string[] = []

      // Check BrowserOS
      try {
        const tools = await deps.browseros.listTools()
        checks.push(
          `✅ **BrowserOS MCP**: Connected (${tools.length} tools available)`,
        )
      } catch (err) {
        checks.push(`❌ **BrowserOS MCP**: Disconnected (${err})`)
      }

      // Check GitButler
      try {
        const status = await deps.gitbutler.getStatus()
        checks.push(
          `✅ **GitButler CLI**: Available (branch: ${status.branch})`,
        )
      } catch (err) {
        checks.push(`❌ **GitButler CLI**: Unavailable (${err})`)
      }

      // Check GitButler MCP
      try {
        if (deps.gitbutler.isConnected) {
          const tools = await deps.gitbutler.listTools()
          checks.push(`✅ **GitButler MCP**: Connected (${tools.length} tools)`)
        } else {
          checks.push(
            `⚠️ **GitButler MCP**: Not connected (will connect on first use)`,
          )
        }
      } catch (err) {
        checks.push(`⚠️ **GitButler MCP**: Connection failed (${err})`)
      }

      checks.push(`\n**Working Dir:** ${deps.config.workingDir}`)
      checks.push(`**Bridge Port:** ${deps.config.port}`)

      return {
        content: [
          {
            type: 'text',
            text: `## TRIOS MCP Bridge Health Check\n\n${checks.join('\n')}`,
          },
        ],
      }
    },
  )

  // ==========================================
  // Tool 12: tri_run (Issue #7)
  // ==========================================
  server.tool(
    'tri_run',
    "Run any `tri` CLI command. Examples: 'test spec.t27', 'verdict', 'status', 'health'. " +
      'Returns stdout, stderr, and exit code. Use for running t27 spec tests, checking verdicts, ' +
      'and managing the PHI LOOP from within BrowserOS chat.',
    {
      command: z
        .string()
        .describe(
          "The tri subcommand and arguments, e.g. 'test spec.t27' or 'verdict'",
        ),
    },
    async (args) => {
      try {
        // Parse command string into args array
        const parts = args.command.trim().split(/\s+/)
        const result = await deps.tri.run(parts)

        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: result.ok,
                exitCode: result.exitCode,
                command: result.command,
                stdout: result.stdout,
                stderr: result.stderr,
              }),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 13: tri_spec_edit (Issue #7)
  // ==========================================
  server.tool(
    'tri_spec_edit',
    'Edit a .t27 spec file and optionally run tests. ' +
      'Writes new content to the spec file, then runs `tri test <path>` for verdict. ' +
      'This closes the PHI LOOP: agent edits spec → gets verdict → commits.',
    {
      specPath: z
        .string()
        .describe('Path to the .t27 spec file relative to working directory'),
      content: z.string().describe('New content for the spec file'),
      runTest: z
        .boolean()
        .default(true)
        .describe('Whether to run `tri test` after writing (default: true)'),
    },
    async (args) => {
      try {
        const result = await deps.tri.specEdit(
          args.specPath,
          args.content,
          args.runTest,
        )

        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: result.ok,
                reason: result.reason,
                specPath: result.specPath,
                testPassed: result.testPassed,
                testOutput: result.testOutput,
              }),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 14: tri_experience_read (Issue #7)
  // ==========================================
  server.tool(
    'tri_experience_read',
    'Read the last N experience entries from .trinity/experience/ files. ' +
      'Returns the most recent .trinity files sorted by modification time. ' +
      'Use to review past session learnings and PHI LOOP history.',
    {
      count: z
        .number()
        .int()
        .min(1)
        .max(50)
        .default(5)
        .describe('Number of recent experience entries to read (default: 5)'),
    },
    async (args) => {
      try {
        const entries = await deps.tri.readExperiences(args.count)

        if (entries.length === 0) {
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({
                  ok: true,
                  reason: 'No .trinity experience files found.',
                  entries: [],
                }),
              },
            ],
          }
        }

        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: true,
                reason: `Found ${entries.length} experience entry/entries.`,
                entries: entries.map(
                  (e: {
                    fileName: string
                    modified: string
                    content: string
                  }) => ({
                    fileName: e.fileName,
                    modified: e.modified,
                    content: e.content,
                  }),
                ),
              }),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 15: Discard File Changes (Issue #14)
  // ==========================================
  server.tool(
    'discard_file_changes',
    'Discard uncommitted changes in specific files. ' +
      'Restores files to their last committed state using `git checkout -- <path>`. ' +
      'Use with caution — discarded changes cannot be recovered.',
    {
      files: z
        .array(z.string())
        .min(1)
        .describe('List of file paths to discard changes for'),
    },
    async (args) => {
      try {
        const _result = await deps.gitbutler.discard(args.files)
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: true,
                reason: `Discarded changes in ${args.files.length} file(s).`,
                files: args.files,
              }),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 16: List Tab Groups (Issue #14)
  // ==========================================
  server.tool(
    'list_tab_groups',
    'List all tab groups in the browser. Returns group IDs, titles, colors, and contained tabs. ' +
      'Requires BrowserOS connection.',
    {},
    async () => {
      try {
        const groups = await deps.browseros.listTabGroups()
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: true,
                groups,
                count: groups.length,
              }),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 17: Create Tab Group (Issue #14)
  // ==========================================
  server.tool(
    'create_tab_group',
    'Create a tab group from given page/tab IDs. Groups tabs together in the browser. ' +
      'Requires BrowserOS connection.',
    {
      page_ids: z
        .array(z.number())
        .min(2)
        .describe('Array of page/tab IDs to group together (minimum 2)'),
      title: z.string().optional().describe('Optional title for the tab group'),
      color: z
        .string()
        .optional()
        .describe(
          "Optional color for the tab group (e.g., 'blue', 'red', 'green')",
        ),
    },
    async (args) => {
      try {
        const result = await deps.browseros.createTabGroup(
          args.page_ids,
          args.title,
          args.color,
        )
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: result.ok,
                reason: result.ok
                  ? `Created tab group with ${args.page_ids.length} tabs.`
                  : 'Failed to create tab group.',
                groupId: result.groupId,
              }),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 18: Search Chapters (RAG)
  // ==========================================
  server.tool(
    'search_chapters',
    'Search GOLDEN BRIDGE chapters by keyword. Returns matching chapter slugs, titles, and snippets. ' +
      'Use for finding specific concepts, theorems, or claims across the Trinity S³AI compendium.',
    {
      query: z.string().min(1).describe('Search keyword or phrase'),
      limit: z
        .number()
        .int()
        .min(1)
        .max(20)
        .default(5)
        .describe('Max results to return'),
    },
    async (args) => {
      try {
        const result = await deps.rag.callTool('search_chapters', {
          query: args.query,
          limit: args.limit,
        })
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 19: Get Chapter (RAG)
  // ==========================================
  server.tool(
    'get_chapter',
    'Fetch the full content of a GOLDEN BRIDGE chapter by slug. ' +
      'Use after search_chapters to read a specific chapter in full.',
    {
      slug: z
        .string()
        .min(1)
        .describe(
          "Chapter slug (e.g., 'phi-squared-identity', 'cp-violation')",
        ),
    },
    async (args) => {
      try {
        const result = await deps.rag.callTool('get_chapter', {
          slug: args.slug,
        })
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 20: List Chapters (RAG)
  // ==========================================
  server.tool(
    'list_chapters',
    'List all GOLDEN BRIDGE chapter slugs with metadata (kind, order, word count). ' +
      'Use for browsing the full catalog before searching.',
    {},
    async () => {
      try {
        const result = await deps.rag.callTool('list_chapters', {})
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 21: Forbidden Audit (RAG)
  // ==========================================
  server.tool(
    'forbidden_audit',
    'Scan all GOLDEN BRIDGE chapters for policy violations and prohibited terms. ' +
      'Returns a safety report used for SSOT discipline verification.',
    {},
    async () => {
      try {
        const result = await deps.rag.callTool('forbidden_audit', {})
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 22: Get Claim Status (RAG)
  // ==========================================
  server.tool(
    'get_claim_status',
    'Search chapters for claim-status markers (Verified, Empirical fit, Open conjecture, High-risk, Falsified, Retracted, Unverified). ' +
      'Returns a per-chapter summary of evidence quality.',
    {
      query: z
        .string()
        .optional()
        .describe('Optional keyword to filter claims'),
    },
    async (args) => {
      try {
        const result = await deps.rag.callTool('get_claim_status', {
          query: args.query || '',
        })
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 23: Build Cover (PhD)
  // ==========================================
  server.tool(
    'build_cover',
    'Generate LaTeX titlepage for the GOLDEN BRIDGE compendium. ' +
      'Returns raw LaTeX that can be compiled with tectonic/pandoc.',
    {},
    async () => {
      try {
        const result = await deps.rag.callTool('build_cover', {})
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 24: Build PDF (PhD)
  // ==========================================
  server.tool(
    'build_pdf',
    'Run the canonical SSOT -> Markdown -> pandoc -> tectonic -> PDF pipeline. ' +
      'DRY-RUN by default — set dryRun=false to actually build. ' +
      'Requires pandoc and tectonic installed locally.',
    {
      dryRun: z
        .boolean()
        .default(true)
        .describe('If true (default), only shows the plan without building.'),
      bookMode: z
        .boolean()
        .default(false)
        .describe('If true, builds with TOC and chapter-level structure.'),
      limit: z
        .number()
        .int()
        .optional()
        .describe('Limit number of chapters to include.'),
      pdfName: z.string().optional().describe('Output PDF filename.'),
      outDir: z.string().optional().describe('Output directory path.'),
    },
    async (args) => {
      try {
        const result = await deps.rag.callTool('build_pdf', {
          dry_run: args.dryRun,
          book_mode: args.bookMode,
          limit: args.limit,
          pdf_name: args.pdfName,
          out_dir: args.outDir,
        })
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 25: List Claims (PhD)
  // ==========================================
  server.tool(
    'list_claims',
    'Scan all GOLDEN BRIDGE chapters for claim-status vocabulary ' +
      '(Verified, Empirical fit, Open conjecture, High-risk, Falsified, Retracted, Unverified). ' +
      'Returns per-chapter summary.',
    {},
    async () => {
      try {
        const result = await deps.rag.callTool('list_claims', {})
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 26: Get Honest Counters (PhD)
  // ==========================================
  server.tool(
    'get_honest_counters',
    'Return the corrected, audited snapshot of trinity-s3ai formal proof counters: ' +
      'machine-checked theorems, open Admitted, axioms, refutation theorems.',
    {},
    async () => {
      try {
        const result = await deps.rag.callTool('get_honest_counters', {})
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 27: Preview Chapter Update (PhD)
  // ==========================================
  server.tool(
    'preview_chapter_update',
    'DRY-RUN only. Show SQL diff and word-count change for a proposed chapter update ' +
      'before committing to Railway PostgreSQL SSOT.',
    {
      slug: z.string().min(1).describe('Chapter slug to preview update for'),
      newTitle: z.string().optional().describe('Proposed new title'),
      newBody: z.string().optional().describe('Proposed new body markdown'),
    },
    async (args) => {
      try {
        const result = await deps.rag.callTool('preview_chapter_update', {
          slug: args.slug,
          new_title: args.newTitle,
          new_body_md: args.newBody,
        })
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 28: Backup SSOT (PhD)
  // ==========================================
  server.tool(
    'backup_ssot',
    'Create a timestamped backup table of the ssot_brochure.chapters table. ' +
      'Requires confirm=true; returns dry-run SQL otherwise. ' +
      'Use before any bulk update or migration.',
    {
      confirm: z
        .boolean()
        .default(false)
        .describe('Set true to execute backup. Default is dry-run.'),
    },
    async (args) => {
      try {
        const result = await deps.rag.callTool('backup_ssot', {
          confirm: args.confirm,
        })
        const text = deps.rag.extractText(result)
        return {
          content: [{ type: 'text', text: text || JSON.stringify(result) }],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 29: Railway Redeploy
  // ==========================================
  server.tool(
    'railway_redeploy',
    'Trigger a redeploy on an existing Railway service. ' +
      'Requires Railway MCP connection.',
    {
      serviceId: z
        .string()
        .optional()
        .describe('Railway service ID to redeploy.'),
      project: z
        .string()
        .optional()
        .describe('Project UUID. Required for multi-account token dispatch.'),
      environment: z
        .string()
        .optional()
        .describe('Environment UUID. Defaults to IGLA production.'),
    },
    async (args) => {
      try {
        if (!deps.railway) {
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({
                  ok: false,
                  reason:
                    'Railway MCP not configured. Set --railway-mcp-url or RAILWAY_MCP_URL.',
                }),
              },
            ],
            isError: true,
          }
        }
        const result = await deps.railway.redeploy(
          args.serviceId,
          args.project,
          args.environment,
        )
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({ ok: true, redeployResult: result }),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 30: Railway Deploy
  // ==========================================
  server.tool(
    'railway_deploy',
    'Create (or reuse) a Railway service, pin its image, upsert env vars, and trigger a redeploy. ' +
      'Requires Railway MCP connection.',
    {
      serviceName: z.string().describe('Name for the new or existing service'),
      image: z
        .string()
        .optional()
        .describe('Docker image to pin (e.g. nginx:latest)'),
      existingServiceId: z
        .string()
        .optional()
        .describe('Reuse an existing service instead of creating a new one'),
      project: z
        .string()
        .optional()
        .describe('Project UUID. Required for multi-account token dispatch.'),
      environment: z
        .string()
        .optional()
        .describe('Environment UUID. Defaults to IGLA production.'),
    },
    async (args) => {
      try {
        if (!deps.railway) {
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({
                  ok: false,
                  reason: 'Railway MCP not configured.',
                }),
              },
            ],
            isError: true,
          }
        }
        const result = await deps.railway.deploy({
          serviceName: args.serviceName,
          image: args.image,
          existingServiceId: args.existingServiceId,
          project: args.project,
          environment: args.environment,
        })
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({ ok: true, deployResult: result }),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 31: Railway List Services
  // ==========================================
  server.tool(
    'railway_list_services',
    'List all Railway services in the IGLA project (or any other project). ' +
      'Returns service IDs, names, and deployment status.',
    {
      project: z
        .string()
        .optional()
        .describe('Project UUID. Defaults to the IGLA project.'),
    },
    async (args) => {
      try {
        if (!deps.railway) {
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({
                  ok: false,
                  reason: 'Railway MCP not configured.',
                }),
              },
            ],
            isError: true,
          }
        }
        const services = await deps.railway.listServices(args.project)
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({ ok: true, services }, null, 2),
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ==========================================
  // Tool 32: Railway Fleet Health
  // ==========================================
  server.tool(
    'railway_fleet_health',
    'Check fleet health across all Railway accounts. ' +
      'Returns service counts, project status, and account connectivity.',
    {},
    async () => {
      try {
        if (!deps.railway) {
          return {
            content: [
              {
                type: 'text',
                text: JSON.stringify({
                  ok: false,
                  reason: 'Railway MCP not configured.',
                }),
              },
            ],
            isError: true,
          }
        }
        const result = await deps.railway.fleetHealth()
        return {
          content: [
            {
              type: 'text',
              text: `## Railway Fleet Health\n\n${result}`,
            },
          ],
        }
      } catch (error) {
        return {
          content: [
            {
              type: 'text',
              text: JSON.stringify({
                ok: false,
                reason: error instanceof Error ? error.message : String(error),
              }),
            },
          ],
          isError: true,
        }
      }
    },
  )

  // ================================================================
  // GitHub Tools (proxied to trios-mcp-github stdio server)
  // ================================================================

  if (deps.github) {
    const gh = deps.github

    function registerGhTool(
      name: string,
      description: string,
      schema: Record<string, z.ZodTypeAny>,
    ) {
      server.tool(name, description, schema, async (args) => {
        try {
          const result = await gh.callTool(
            name,
            args as Record<string, unknown>,
          )
          return {
            content: [{ type: 'text' as const, text: gh.extractText(result) }],
          }
        } catch (error) {
          return {
            content: [
              {
                type: 'text' as const,
                text: JSON.stringify({
                  ok: false,
                  reason:
                    error instanceof Error ? error.message : String(error),
                }),
              },
            ],
            isError: true,
          }
        }
      })
    }

    registerGhTool('github_repo_info', 'Get repository metadata.', {
      owner: z.string(),
      repo: z.string(),
    })
    registerGhTool('github_read_file', 'Read a file from a repository.', {
      owner: z.string(),
      repo: z.string(),
      path: z.string(),
      ref: z.string().optional(),
    })
    registerGhTool(
      'github_list_files',
      'List files and directories in a repo path.',
      {
        owner: z.string(),
        repo: z.string(),
        path: z.string().optional(),
        ref: z.string().optional(),
      },
    )
    registerGhTool('github_list_issues', 'List issues in a repository.', {
      owner: z.string(),
      repo: z.string(),
      state: z.enum(['open', 'closed', 'all']).optional(),
      per_page: z.number().int().min(1).max(100).optional(),
    })
    registerGhTool(
      'github_create_issue',
      'Create an issue. Dry-run by default.',
      {
        owner: z.string(),
        repo: z.string(),
        title: z.string(),
        body: z.string().optional(),
        labels: z.array(z.string()).optional(),
        dry_run: z.boolean().optional(),
      },
    )
    registerGhTool(
      'github_create_pr',
      'Create a pull request. Dry-run by default.',
      {
        owner: z.string(),
        repo: z.string(),
        title: z.string(),
        head: z.string(),
        base: z.string().optional(),
        body: z.string().optional(),
        draft: z.boolean().optional(),
        dry_run: z.boolean().optional(),
      },
    )
    registerGhTool('github_list_commits', 'List recent commits.', {
      owner: z.string(),
      repo: z.string(),
      path: z.string().optional(),
      per_page: z.number().int().min(1).max(100).optional(),
    })
    registerGhTool('github_search_code', 'Search code across GitHub.', {
      query: z.string(),
      per_page: z.number().int().min(1).max(100).optional(),
    })
    registerGhTool('github_list_branches', 'List branches in a repository.', {
      owner: z.string(),
      repo: z.string(),
      per_page: z.number().int().min(1).max(100).optional(),
    })
    registerGhTool('github_get_workflow_status', 'Get workflow run status.', {
      owner: z.string(),
      repo: z.string(),
      workflow_id: z.string().optional(),
      per_page: z.number().int().min(1).max(30).optional(),
    })
    registerGhTool(
      'github_add_comment',
      'Add a comment to an issue or PR. Dry-run by default.',
      {
        owner: z.string(),
        repo: z.string(),
        issue_number: z.number().int(),
        body: z.string(),
        dry_run: z.boolean().optional(),
      },
    )
    registerGhTool('github_list_pulls', 'List pull requests.', {
      owner: z.string(),
      repo: z.string(),
      state: z.enum(['open', 'closed', 'all']).optional(),
      per_page: z.number().int().min(1).max(100).optional(),
    })
  }

  return server
}
