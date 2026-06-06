import { z } from 'zod'
import { defineTool } from '../framework'

export const gitbutler_vision_context = defineTool({
  name: 'gitbutler_vision_context',
  description:
    "Get comprehensive Git repository context including files, branches, commits, and UI state for LLM analysis. Provides a 'memory' of repository state suitable for agents to understand what has been changed without requiring file system access.",
  approvalCategory: 'automation',
  input: z.object({
    include_ui_state: z
      .boolean()
      .optional()
      .describe(
        'Include GitButler UI state information (active panel, modals, selections, etc.)',
      ),
    include_changes: z
      .boolean()
      .optional()
      .describe(
        'Include detailed change information (what files changed, which commits touched what)',
      ),
    max_files: z
      .number()
      .min(1)
      .max(50)
      .optional()
      .default(20)
      .describe(
        'Maximum number of files to return (for large repos, limit to avoid token overflow)',
      ),
    include_recent_commits: z
      .number()
      .min(1)
      .max(10)
      .optional()
      .default(5)
      .describe('Include recent commit history (0=all, 5=last 5)'),
    include_branches: z
      .boolean()
      .optional()
      .describe('Include all branch information'),
  }),
  output: z.object({
    repository_info: z.object({
      name: z
        .string()
        .describe("Repository name (e.g., 'trinity' or user/repo name)"),
      head: z.string().describe('Current HEAD commit SHA'),
      branch_count: z.number().describe('Total number of branches'),
      clean: z
        .boolean()
        .describe(
          'Whether working directory is clean (no uncommitted changes)',
        ),
      total_files: z.number().describe('Total tracked files'),
      recent_commit_count: z
        .number()
        .describe('Number of commits in default branch'),
      total_commits: z.number().describe('Total commits across all branches'),
    }),
    ui_state: z
      .object({
        active_panel: z
          .string()
          .describe(
            "Currently active panel (e.g., 'scm', 'ai-chat', 'history')",
          ),
        active_modal: z
          .string()
          .optional()
          .describe("Currently open modal (e.g., 'settings', 'create-branch')"),
        branch_name: z
          .string()
          .optional()
          .describe('Name of current or last checked-out branch'),
        branch_status: z
          .string()
          .optional()
          .describe("Branch status: 'clean', 'ahead', 'behind', 'detached')"),
        staged_files: z
          .number()
          .optional()
          .describe('Number of files staged for commit'),
        visible_files: z
          .array(z.string())
          .describe("List of files marked as 'visible' in GitButler UI"),
        is_loading: z
          .boolean()
          .optional()
          .describe('Whether GitButler is currently loading repository state'),
      })
      .describe('GitButler UI and repository state'),
    changes: z
      .array(
        z.object({
          type: z.enum([
            'file_added',
            'file_modified',
            'file_deleted',
            'file_renamed',
          ]),
          path: z.string().describe('File path (relative to repository root)'),
          status: z
            .enum(['visible', 'hidden'])
            .optional()
            .describe('Visibility in UI'),
          branch_name: z
            .string()
            .optional()
            .describe('Branch name for this change (if applicable)'),
          commit_info: z
            .object({
              sha: z.string().optional().describe('Git commit SHA (short)'),
              message: z.string().optional().describe('Commit message'),
              timestamp: z.string().optional().describe('When commit was made'),
            })
            .optional()
            .describe('Commit details (for file changes)'),
        }),
      )
      .describe('List of changes detected in repository'),
    recent_commits: z
      .array(
        z.object({
          sha: z.string().describe('Git commit SHA (short)'),
          message: z.string().describe('Commit message'),
          timestamp: z.string().describe('When commit was made'),
          branch: z.string().optional().describe('Branch name'),
          author: z.string().optional().describe('Commit author'),
        }),
      )
      .describe('Recent commit history'),
  }),
  handler: async (_args, _ctx, response) => {
    // Phase 2 implementation: Call GitButler MCP tools and aggregate data
    // TODO: Replace placeholder implementation with actual LLM calls

    response.text('Retrieving GitButler repository context...')

    // Placeholder response - in real implementation would call:
    // 1. GitButler vision tool (screenshot UI) → get PNG → decode to files
    // 2. GitButler branch tool → get current branch, list branches
    // 3. GitButler changes tool → get file changes, commits
    // 4. GitButler staging tool → get/list, stage/unstage files

    response.data({
      repository_info: {
        name: 'BrowserOS', // TODO: get from gitbutler config
        head: 'unknown', // TODO: get from git status
        branch_count: 1,
        clean: true,
        total_files: 0,
        recent_commit_count: 0,
        total_commits: 0,
      },
      ui_state: {
        active_panel: 'ai-chat',
        visible_files: [],
        is_loading: false,
      },
      changes: [
        { type: 'file_modified', path: 'README.md', status: 'visible' },
      ],
      recent_commits: [],
    })

    response.image?.(
      'iVBORw0lGkq6QAAAADAAAAAAwAAAAAEAAABAAAAAAAAADAAAAAwAAAAAMAAAAA==',
      'image/png',
    )
  },
})
