import { z } from "zod";
import { defineTool } from "../framework";

// Stage all visible changes in GitButler
export const stage_visible_changes = defineTool({
	name: "stage_visible_changes",
	description:
		"Stage all files that are marked as 'visible' or 'modified' in GitButler into the current branch",
	approvalCategory: "automation",
	input: z.object({}),
	output: z.object({
		success: z.boolean(),
		staged_files: z.array(z.string()),
		branch_name: z.string().optional(),
		message: z.string().optional(),
	}),
	handler: async (_args, _ctx, response) => {
		response.text("Staging visible GitButler changes...");

		// TODO: Call GitButler MCP tool
		// For now, return placeholder
		response.data({
			success: true,
			staged_files: [],
			branch_name: null,
			message: "Not implemented yet. Use GitButler MCP tool.",
		});
	},
});

// Commit changes with a custom message
export const commit_with_message = defineTool({
	name: "commit_with_message",
	description: "Create a Git commit in GitButler with a custom commit message",
	approvalCategory: "automation",
	input: z.object({
		message: z
			.string()
			.describe("Commit message (e.g., 'fix: navigation bug')"),
		file_list: z
			.array(z.string())
			.optional()
			.describe("List of files to commit (comma-separated)"),
	}),
	output: z.object({
		success: z.boolean(),
		sha: z.string().describe("Git commit SHA"),
		message: z.string().describe("Commit message from GitButler"),
		branch_name: z.string().optional().describe("Branch name"),
	}),
	handler: async (_args, _ctx, response) => {
		response.text("Creating Git commit with message...");

		// TODO: Call GitButler MCP tool
		// For now, return placeholder
		response.data({
			success: true,
			sha: "placeholder",
			message: "Not implemented yet. Use GitButler MCP tool.",
		});
	},
});

// Create a new branch from current state
export const create_branch = defineTool({
	name: "create_branch",
	description:
		"Create a new Git branch in GitButler from the current branch name (e.g., 'feature/xyz')",
	approvalCategory: "automation",
	input: z.object({
		branch_name: z.string().describe("Branch name (e.g., 'feature/xyz')"),
	}),
	output: z.object({
		success: z.boolean(),
		branch_name: z.string().describe("Created branch name"),
		message: z.string().describe("Status message from GitButler"),
	}),
	handler: async (_args, _ctx, response) => {
		response.text("Creating new Git branch...");

		// TODO: Call GitButler MCP tool
		// For now, return placeholder
		response.data({
			success: true,
			branch_name: _args.branch_name,
			message: "Not implemented yet. Use GitButler MCP tool.",
		});
	},
});

// Push current branch to remote
export const push_branch = defineTool({
	name: "push_branch",
	description: "Push the current Git branch to remote in GitButler",
	approvalCategory: "automation",
	input: z.object({}),
	output: z.object({
		success: z.boolean(),
		branch_name: z.string().describe("Branch name"),
		message: z.string().describe("Status message from GitButler"),
		remote_url: z
			.string()
			.optional()
			.describe("Remote URL (e.g., 'https://github.com/user/repo')"),
	}),
	handler: async (_args, _ctx, response) => {
		response.text("Pushing Git branch...");

		// TODO: Call GitButler MCP tool
		// For now, return placeholder
		response.data({
			success: true,
			branch_name: null,
			message: "Not implemented yet. Use GitButler MCP tool.",
		});
	},
});
