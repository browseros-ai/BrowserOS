/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS MCP Bridge - Take GitButler Screenshot
 * Uses BrowserOS MCP to capture a screenshot of the GitButler UI.
 */

import { z } from "zod";
import type { BrowserOSClient } from "../clients/browseros-client.js";
import { defineTool } from "../framework";

export const take_gitbutler_screenshot = defineTool({
	name: "take_gitbutler_screenshot",
	description:
		"Take a screenshot of the GitButler UI window via BrowserOS MCP and return it as base64 PNG data.",
	approvalCategory: "automation",
	input: z.object({}),
	output: z.object({
		image_data: z.string().describe("Base64-encoded PNG image data"),
		mimeType: z.string().describe("MIME type (e.g., 'image/png')"),
	}),
	handler: async (_args, ctx, response) => {
		response.text("Capturing GitButler screenshot...");

		try {
			const browserosClient = ctx.clients.browseros as BrowserOSClient;

			// Find GitButler page
			const page = await browserosClient.findGitButlerPage();
			if (!page) {
				response.error(
					"GitButler tab not found. Please open GitButler in the browser first.",
				);
				return;
			}

			// Take screenshot
			const screenshot = await browserosClient.takeScreenshot(page.id);

			if (screenshot.status === "fulfilled") {
				response.data({
					image_data: screenshot.value.data,
					mimeType: screenshot.value.mimeType || "image/png",
				});
				response.image(screenshot.value.data, screenshot.value.mimeType);
			} else {
				response.error(
					`Screenshot failed: ${screenshot.reason || "Unknown error"}`,
				);
			}
		} catch (error) {
			response.error(
				`Error taking screenshot: ${error instanceof Error ? error.message : String(error)}`,
			);
		}
	},
});
