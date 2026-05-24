/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * git_status tool — Get current Git repository status
 * Uses simple-git with proper type casts.
 */

import { z } from "zod";
import { defineTool } from "../framework";

export const git_status = defineTool({
	name: "git_status",
	description:
		"Get current Git repository status (branch, modified files, staged files, recent commits). " +
		"Useful for understanding development state before making changes.",
	approvalCategory: "automation",
	input: z.object({}),
	output: z.object({
		current_branch: z.string().describe("Current branch name"),
		is_clean: z
			.boolean()
			.describe("Whether working directory is clean (no uncommitted changes)"),
		modified_files: z
			.array(
				z.object({
					filename: z.string(),
					status: z.enum(["added", "modified", "deleted"]),
				}),
			)
			.describe("List of modified/untracked files"),
		staged_files: z.array(z.string()).describe("List of staged files"),
		recent_commits: z
			.array(
				z.object({
					sha: z.string(),
					message: z.string(),
					timestamp: z.string(),
				}),
			)
			.describe("Recent commits"),
		head_commit: z
			.object({
				sha: z.string(),
				message: z.string(),
			})
			.describe("HEAD commit information"),
	}),
	handler: async (_args, ctx, response) => {
		response.text("Git repository status:");

		try {
			const gitDir = ctx.directories?.workingDir || process.cwd();

			// Use raw git commands via Bun.spawnSync (no simple-git dependency)
			const branchProc = Bun.spawnSync(
				["git", "rev-parse", "--abbrev-ref", "HEAD"],
				{ cwd: gitDir },
			);
			const currentBranch =
				branchProc.exitCode === 0
					? branchProc.stdout.toString().trim()
					: "unknown";

			// Get porcelain status
			const statusProc = Bun.spawnSync(
				["git", "status", "--porcelain=v2", "--branch"],
				{ cwd: gitDir },
			);

			const modifiedFiles: Array<{
				filename: string;
				status: "added" | "modified" | "deleted";
			}> = [];
			const stagedFiles: string[] = [];
			let isClean = true;

			if (statusProc.exitCode === 0) {
				const lines = statusProc.stdout.toString().split("\n");
				for (const line of lines) {
					if (line.startsWith("1 ")) {
						// Changed entry: 1 <xy> <sub> <mH> <mI> <mW> <hH> <hI> <path>
						const parts = line.split(" ");
						if (parts.length < 9) continue;
						const xy = parts[1];
						const filePath = parts.slice(8).join(" ");

						const indexStatus = xy[0];
						const workTreeStatus = xy[1];

						// Staged changes (index)
						if (indexStatus && indexStatus !== "." && indexStatus !== "?") {
							stagedFiles.push(filePath);
						}

						// Modified/untracked (work tree)
						if (
							workTreeStatus &&
							workTreeStatus !== "." &&
							workTreeStatus !== "?"
						) {
							const statusMap: Record<
								string,
								"added" | "modified" | "deleted"
							> = {
								M: "modified",
								A: "added",
								D: "deleted",
								R: "modified",
								C: "added",
							};
							modifiedFiles.push({
								filename: filePath,
								status: statusMap[workTreeStatus] || "modified",
							});
							isClean = false;
						}

						if (indexStatus && indexStatus !== ".") {
							isClean = false;
						}
					} else if (line.startsWith("? ")) {
						// Untracked file
						const filePath = line.slice(2);
						modifiedFiles.push({ filename: filePath, status: "added" });
						isClean = false;
					}
				}
			}

			// Get recent commits (last 5)
			const logProc = Bun.spawnSync(
				["git", "log", "-5", "--format=%H|%s|%ct"],
				{ cwd: gitDir },
			);

			const recentCommits: Array<{
				sha: string;
				message: string;
				timestamp: string;
			}> = [];

			let headCommit = { sha: "unknown", message: "unknown" };

			if (logProc.exitCode === 0) {
				const logLines = logProc.stdout.toString().trim().split("\n");
				for (const logLine of logLines) {
					if (!logLine) continue;
					const [sha, message, timestamp] = logLine.split("|");
					const entry = {
						sha: sha?.slice(0, 7) || "unknown",
						message: message || "unknown",
						timestamp: timestamp
							? new Date(Number(timestamp) * 1000).toISOString()
							: new Date().toISOString(),
					};
					recentCommits.push(entry);
				}
				if (recentCommits.length > 0) {
					headCommit = {
						sha: recentCommits[0].sha,
						message: recentCommits[0].message,
					};
				}
			}

			response.data({
				current_branch: currentBranch,
				is_clean: isClean,
				modified_files: modifiedFiles,
				staged_files: stagedFiles,
				recent_commits: recentCommits,
				head_commit: headCommit,
			});
		} catch (error) {
			response.error(
				`Failed to get git status: ${error instanceof Error ? error.message : String(error)}`,
			);
		}
	},
});
