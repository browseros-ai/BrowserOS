/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * BrowserOS MCP Client
 * Connects to BrowserOS MCP server for screenshots, snapshots, and browser control.
 */

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import type { ScreenshotResult, SnapshotResult } from "../types.js";

export class BrowserOSClient {
	private client: Client | null = null;
	private serverUrl: string;

	constructor(serverUrl: string) {
		this.serverUrl = serverUrl;
	}

	async connect(): Promise<void> {
		this.client = new Client({
			name: "trios-mcp-bridge",
			version: "0.1.0",
		});

		const transport = new StreamableHTTPClientTransport(
			new URL(this.serverUrl),
		);

		await this.client.connect(transport);
		console.log(`[BrowserOS] Connected to ${this.serverUrl}`);
	}

	async disconnect(): Promise<void> {
		if (this.client) {
			await this.client.close();
			this.client = null;
			console.log("[BrowserOS] Disconnected");
		}
	}

	get isConnected(): boolean {
		return this.client !== null;
	}

	/** Ensure client is connected, reconnect if needed */
	private async ensureConnected(): Promise<Client> {
		if (!this.client) {
			await this.connect();
		}
		return this.client!;
	}

	/** List all open pages/tabs */
	async listPages(): Promise<
		Array<{ id: number; url: string; title: string }>
	> {
		const client = await this.ensureConnected();
		const result = await client.callTool({
			name: "browser_list_pages",
			arguments: {},
		});

		const text = this.extractText(result);
		return this.parsePagesList(text);
	}

	/** Find a page that looks like GitButler */
	async findGitButlerPage(): Promise<{
		id: number;
		url: string;
		title: string;
	} | null> {
		const pages = await this.listPages();
		return (
			pages.find(
				(p) =>
					p.url.includes("gitbutler") ||
					p.title.toLowerCase().includes("gitbutler"),
			) ?? null
		);
	}

	/** Take a screenshot of a specific page */
	async takeScreenshot(pageId: number): Promise<ScreenshotResult> {
		const client = await this.ensureConnected();
		const result = await client.callTool({
			name: "browser_take_screenshot",
			arguments: {
				page: pageId,
				format: "png",
			},
		});

		// Extract image data from MCP result
		const imageContent = result.content?.find((c: any) => c.type === "image");
		if (imageContent?.data) {
			return {
				data: imageContent.data,
				mimeType: imageContent.mimeType || "image/png",
				devicePixelRatio: 1,
			};
		}

		// Fallback: try take_screenshot tool name
		const result2 = await client.callTool({
			name: "take_screenshot",
			arguments: {
				page: pageId,
				format: "png",
			},
		});

		const imageContent2 = result2.content?.find((c: any) => c.type === "image");
		if (imageContent2?.data) {
			return {
				data: imageContent2.data,
				mimeType: imageContent2.mimeType || "image/png",
				devicePixelRatio: 1,
			};
		}

		throw new Error("No screenshot data returned from BrowserOS");
	}

	/** Take an accessibility snapshot of a page */
	async takeSnapshot(pageId: number): Promise<SnapshotResult> {
		const client = await this.ensureConnected();
		const result = await client.callTool({
			name: "browser_take_snapshot",
			arguments: { page: pageId },
		});

		const text = this.extractText(result);
		return { snapshot: text };
	}

	/** Get page content as markdown */
	async getPageContent(pageId: number): Promise<string> {
		const client = await this.ensureConnected();
		const result = await client.callTool({
			name: "browser_get_page_content",
			arguments: { page: pageId },
		});

		return this.extractText(result);
	}

	/** Click at coordinates on a page */
	async clickAt(pageId: number, x: number, y: number): Promise<string> {
		const client = await this.ensureConnected();
		const result = await client.callTool({
			name: "browser_click_at",
			arguments: { page: pageId, x, y },
		});
		return this.extractText(result);
	}

	/** Navigate to a URL */
	async navigate(pageId: number, url: string): Promise<string> {
		const client = await this.ensureConnected();
		const result = await client.callTool({
			name: "browser_navigate",
			arguments: { page: pageId, url },
		});
		return this.extractText(result);
	}

	/** List available tools from BrowserOS MCP */
	async listTools(): Promise<string[]> {
		const client = await this.ensureConnected();
		const result = await client.listTools();
		return result.tools.map((t) => t.name);
	}

	// --- Helpers ---

	/** List tab groups in the browser */
	async listTabGroups(): Promise<
		Array<{ id: number; title: string; color: string; pageIds: number[] }>
	> {
		const client = await this.ensureConnected();
		try {
			const result = await client.callTool({
				name: "browser_list_tab_groups",
				arguments: {},
			});
			const text = this.extractText(result);
			return this.parseTabGroups(text);
		} catch {
			return [];
		}
	}

	/** Create a tab group from given page IDs */
	async createTabGroup(
		pageIds: number[],
		title?: string,
		color?: string,
	): Promise<{ ok: boolean; groupId?: number }> {
		const client = await this.ensureConnected();
		try {
			const args: Record<string, unknown> = { page_ids: pageIds };
			if (title) args.title = title;
			if (color) args.color = color;

			const result = await client.callTool({
				name: "browser_group_tabs",
				arguments: args,
			});
			const text = this.extractText(result);
			const match = text.match(/group.*?(\d+)/i);
			return { ok: true, groupId: match ? Number(match[1]) : undefined };
		} catch {
			return { ok: false };
		}
	}

	private extractText(result: any): string {
		if (!result.content) return "";
		return result.content
			.filter((c: any) => c.type === "text")
			.map((c: any) => c.text)
			.join("\n");
	}

	private parsePagesList(
		text: string,
	): Array<{ id: number; url: string; title: string }> {
		const pages: Array<{ id: number; url: string; title: string }> = [];
		// Parse the page list format from BrowserOS
		const lines = text.split("\n");
		for (const line of lines) {
			const match = line.match(/\[(\d+)\]\s+(.+?)\s+(https?:\/\/\S+)/);
			if (match) {
				pages.push({
					id: Number(match[1]),
					title: match[2].trim(),
					url: match[3].trim(),
				});
			}
		}
		return pages;
	}

	private parseTabGroups(
		text: string,
	): Array<{ id: number; title: string; color: string; pageIds: number[] }> {
		const groups: Array<{
			id: number;
			title: string;
			color: string;
			pageIds: number[];
		}> = [];
		try {
			// Try JSON parse first
			const parsed = JSON.parse(text);
			if (Array.isArray(parsed)) return parsed;
		} catch {
			// Fall through to text parsing
		}
		// Simple text parsing fallback
		const lines = text.split("\n");
		for (const line of lines) {
			const match = line.match(
				/group.*?(\d+).*?title[:\s]+(\w+).*?color[:\s]+(\w+)/i,
			);
			if (match) {
				groups.push({
					id: Number(match[1]),
					title: match[2],
					color: match[3],
					pageIds: [],
				});
			}
		}
		return groups;
	}
}
