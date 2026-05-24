/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS MCP Bridge - Analyze GitButler UI (Vision)
 * Uses vision analyzer + CLI status to understand GitButler UI state.
 */

import { z } from "zod";
import type { GitButlerMcpClient } from "../clients/gitbutler-client.js";
import { defineTool } from "../framework";
import { analyzeGitButlerUI } from "../vision/gitbutler-analyzer.js";

export const analyze_gitbutler_ui = defineTool({
	name: "analyze_gitbutler_ui",
	description:
		"Analyze a GitButler UI screenshot (file + screenshot) and return structured data about changes, branches, and UI state. Uses vision analyzer + CLI status for accuracy.",
	approvalCategory: "automation",
	input: z.object({
		screenshot: z
			.string()
			.describe("Base64-encoded PNG screenshot of GitButler UI"),
	}),
	output: z.object({
		files_detected: z
			.array(
				z.object({
					filename: z.string(),
					status: z.enum(["visible", "added", "modified", "deleted"]),
				}),
			)
			.describe("Files detected in screenshot with their status"),
		branch_info: z
			.object({
				name: z.string(),
				status: z.enum(["clean", "ahead", "behind", "detached"]),
			})
			.describe("Branch information"),
		ui_state: z
			.object({
				active_panel: z
					.string()
					.optional()
					.describe("Active side panel (e.g., 'scm', 'ai-chat')"),
				modal_open: z.boolean().optional().describe("Modal dialog is open"),
			})
			.describe("GitButler UI state information"),
		suggestions: z
			.array(z.string())
			.describe(
				"AI suggestions for next actions (staging files, creating branches, etc.)",
			),
	}),
	handler: async (args, ctx, response) => {
		response.text("Analyzing GitButler UI with vision analyzer...");

		try {
			const gitbutlerClient = ctx.clients.gitbutler as GitButlerMcpClient;

			const cliStatus = await gitbutlerClient.getStatus();

			const analysis = analyzeGitButlerUI(args.screenshot, cliStatus);

			response.data({
				files_detected: analysis.changedFiles,
				branch_info: {
					name: analysis.activeBranch,
					status: analysis.isClean
						? "clean"
						: analysis.activeBranch !== "main"
							? "dirty"
							: "clean",
				},
				ui_state: {
					active_panel: "unknown",
					modal_open: false,
				},
				suggestions: analysis.suggestedActions,
			});
		} catch (error) {
			response.error(
				`Failed to analyze GitButler UI: ${error instanceof Error ? error.message : String(error)}`,
			);
		}
	},
});
