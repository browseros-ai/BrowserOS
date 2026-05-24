/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS MCP Bridge - Commit Visible Changes
 * High-level tool to commit currently visible/modified files via GitButler.
 */

import { z } from "zod";
import type { GitButlerMcpClient } from "../clients/gitbutler-client.js";
import { defineTool } from "../framework";

export const commit_visible_changes = defineTool({
	name: "commit_visible_changes",
	description:
		"Commit currently visible/modified files with a descriptive message. Uses GitButler's smart commit which auto-stages changed files.",
	approvalCategory: "automation",
	input: z.object({
		message: z
			.string()
			.min(1)
			.max(2048)
			.describe("Commit message describing the changes"),
	}),
	output: z.object({
		success: z.boolean(),
		sha: z.string().describe("Git commit SHA"),
		message: z.string().describe("Commit message from GitButler"),
		branch_name: z.string().describe("Branch name"),
		files_committed: z.array(z.string()).describe("Files that were committed"),
	}),
	handler: async (args, ctx, response) => {
		response.text("Committing visible changes...");

		try {
			const gitbutlerClient = ctx.clients.gitbutler as GitButlerMcpClient;

			const status = await gitbutlerClient.getStatus();

			if (status.staged.length === 0 && status.unstaged.length === 0) {
				response.error("No changes to commit. Make some modifications first.");
				return;
			}

			const filesToStage: string[] = [];

			if (status.staged.length === 0) {
				filesToStage.push(...status.unstaged.map((f) => f.path));
			} else {
				filesToStage.push(...status.staged.map((f) => f.path));
			}

			if (filesToStage.length > 0) {
				await gitbutlerClient.stage(filesToStage);
			}

			const result = await gitbutlerClient.commit(args.message);

			if (result.success) {
				response.data({
					success: true,
					sha: result.hash,
					message: args.message,
					branch_name: status.branch,
					files_committed: filesToStage,
				});
			} else {
				response.error(`Commit failed: ${result.error}`);
			}
		} catch (error) {
			response.error(
				`Error committing: ${error instanceof Error ? error.message : String(error)}`,
			);
		}
	},
});
