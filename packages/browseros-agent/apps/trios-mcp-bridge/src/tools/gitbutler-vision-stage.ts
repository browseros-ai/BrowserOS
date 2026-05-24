import { z } from "zod";
import { defineTool } from "../framework";

// Stage all visible changes in GitButler
export const stage_visible_changes = defineTool({
	name: "git_stage_files",
	description:
		"Stage all files that are marked as 'visible' or 'modified' in GitButler into the current branch. Returns list of staged files and branch info.",
	approvalCategory: "automation",
	input: z.object({
		paths: z
			.array(z.string())
			.describe("List of file paths to stage (comma-separated)"),
	}),
	output: z.object({
		success: z.boolean().describe("True if staging succeeded"),
		staged_files: z
			.array(
				z.object({
					filename: z.string(),
					status: z.enum(["visible", "added", "modified", "deleted"]),
				}),
			)
			.describe("List of files that were staged"),
		branch_name: z.string().optional().describe("Current branch name"),
		branch_status: z
			.enum(["clean", "ahead", "behind"])
			.optional()
			.describe("Branch status relative to remote"),
	}),
	handler: async (_args, _ctx, response) => {
		// TODO: Call GitButler MCP tool
		response.text("GitButler stage_files not implemented yet");
		response.data({
			success: false,
			staged_files: [],
			branch_name: null,
			branch_status: null,
			suggestions: ["Use GitButler MCP tool: git_stage_files"],
		});
	},
});

// Commit changes with a custom message
export const commit_with_message = defineTool({
	name: "git_commit",
	description:
		"Create a Git commit in GitButler with a custom commit message. Returns commit SHA and branch info.",
	approvalCategory: "automation",
	input: z.object({
		message: z
			.string()
			.describe("Commit message (e.g., 'fix: navigation bug')"),
		paths: z
			.array(z.string())
			.optional()
			.describe("List of file paths to commit (comma-separated)"),
	}),
	output: z.object({
		success: z.boolean().describe("True if commit succeeded"),
		sha: z.string().describe("Git commit SHA"),
		branch_name: z.string().optional().describe("Branch name after commit"),
		message: z.string().describe("Actual commit message from GitButler"),
	}),
	handler: async (_args, _ctx, response) => {
		// TODO: Call GitButler MCP tool
		response.text("GitButler commit not implemented yet");
		response.data({
			success: false,
			sha: "placeholder",
			branch_name: null,
			message: "Not implemented yet",
		});
	},
});

// Create a new branch from current state
export const create_branch = defineTool({
	name: "git_create_branch",
	description:
		"Create a new Git branch in GitButler from the current branch state (detected from UI). Returns branch name and status.",
	approvalCategory: "automation",
	input: z.object({
		branch_name: z.string().describe("Branch name (e.g., 'feature/xyz')"),
	}),
	output: z.object({
		success: z.boolean().describe("True if branch created successfully"),
		branch_name: z.string().describe("Created branch name"),
		branch_status: z
			.enum(["clean", "ahead", "behind"])
			.optional()
			.describe("Branch status relative to remote"),
	}),
	handler: async (_args, _ctx, response) => {
		// TODO: Call GitButler MCP tool
		response.text("GitButler create_branch not implemented yet");
		response.data({
			success: false,
			branch_name: null,
			branch_status: null,
			suggestions: ["Use GitButler MCP tool: git_create_branch"],
		});
	},
});

// Push current branch to remote
export const push_branch = defineTool({
	name: "git_push_branch",
	description:
		"Push the current Git branch to the remote repository. Returns push status and branch name.",
	approvalCategory: "automation",
	input: z.object({}),
	output: z.object({
		success: z.boolean().describe("True if push succeeded"),
		branch_name: z.string().optional().describe("Branch name that was pushed"),
	}),
	handler: async (_args, _ctx, response) => {
		// TODO: Call GitButler MCP tool
		response.text("GitButler push_branch not implemented yet");
		response.data({
			success: false,
			branch_name: null,
			suggestions: ["Use GitButler MCP tool: git_push_branch"],
		});
	},
});
