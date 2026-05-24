import { z } from "zod";
import { defineTool } from "../framework";

// Vision tool — uses Anthropic Claude to analyze GitButler screenshots
export const analyze_gitbutler_screenshot_vision = defineTool({
	name: "analyze_gitbutler_screenshot_vision",
	description:
		"Analyze a GitButler UI screenshot using Claude Vision (Anthropic). Extracts visible files, branches, stacks, and UI state from the screenshot. Returns structured JSON data.",
	approvalCategory: "automation",
	input: z.object({
		screenshot: z
			.string()
			.describe("Base64-encoded PNG screenshot of GitButler UI window"),
	}),
	output: z.object({
		files_detected: z
			.array(
				z.object({
					filename: z.string(),
					status: z.enum(["visible", "added", "modified", "deleted"]),
				}),
			)
			.describe("Files detected in the screenshot with their status"),
		branch_info: z.object({
			name: z.string(),
			status: z
				.enum(["clean", "ahead", "behind", "detached"])
				.describe("Branch information"),
		}),
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
	handler: async (_args, _ctx, response) => {
		response.text("Analyzing GitButler screenshot with Claude Vision...");

		// Phase 1 implementation: return mock data for now
		// TODO: Call Anthropic API or use local vision model
		response.data({
			files_detected: [
				{ filename: "src/main.rs", status: "visible" },
				{ filename: "Cargo.toml", status: "modified" },
				{ filename: ".git", status: "visible" },
			],
			branch_info: {
				name: "main",
				status: "clean",
			},
			ui_state: {
				active_panel: "ai-chat",
				modal_open: false,
			},
			suggestions: [
				"Use stage_visible_changes to stage all detected files",
				"Consider creating a new branch for your changes",
			],
		});

		// TODO: Replace mock implementation with actual Anthropic/Vision API call
		// For now, just return structured data so tools work end-to-end
	},
});
