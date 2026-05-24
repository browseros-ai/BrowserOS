/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * GitButler MCP Client
 * Connects to GitButler's `but mcp` MCP server via stdio subprocess.
 */

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import type {
	BranchInfo,
	CommitResult,
	FileChange,
	GitButlerStatus,
} from "../types.js";

export class GitButlerMcpClient {
	private client: Client | null = null;
	private transport: StdioClientTransport | null = null;
	private cliPath: string;
	private internal: boolean;
	private workingDir: string;

	constructor(cliPath: string, internal: boolean, workingDir: string) {
		this.cliPath = cliPath;
		this.internal = internal;
		this.workingDir = workingDir;
	}

	async connect(): Promise<void> {
		const args = ["mcp"];
		if (this.internal) {
			args.push("--internal");
		}

		this.transport = new StdioClientTransport({
			command: this.cliPath,
			args,
			cwd: this.workingDir,
		});

		this.client = new Client({
			name: "trios-mcp-bridge",
			version: "0.1.0",
		});

		await this.client.connect(this.transport);
		const mode = this.internal ? "internal" : "simple";
		console.log(`[GitButler] Connected via MCP (${mode} mode)`);
	}

	async disconnect(): Promise<void> {
		if (this.client) {
			await this.client.close();
			this.client = null;
		}
		if (this.transport) {
			await this.transport.close();
			this.transport = null;
		}
		console.log("[GitButler] Disconnected");
	}

	get isConnected(): boolean {
		return this.client !== null;
	}

	private async ensureConnected(): Promise<Client> {
		if (!this.client) {
			await this.connect();
		}
		return this.client!;
	}

	/** List available tools from GitButler MCP */
	async listTools(): Promise<string[]> {
		const client = await this.ensureConnected();
		const result = await client.listTools();
		return result.tools.map((t) => t.name);
	}

	/** Get workspace status via `but status` */
	async getStatus(): Promise<GitButlerStatus> {
		// Use CLI directly for reliable status
		const proc = Bun.spawnSync([this.cliPath, "status", "--json"], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			// Fallback to git status
			return this.getGitStatusFallback();
		}

		try {
			const output = proc.stdout.toString().trim();
			const data = JSON.parse(output);
			return this.parseButStatus(data);
		} catch {
			return this.getGitStatusFallback();
		}
	}

	/** List branches */
	async getBranches(): Promise<BranchInfo[]> {
		const proc = Bun.spawnSync([this.cliPath, "branch", "list", "--json"], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			return this.getGitBranchesFallback();
		}

		try {
			const output = proc.stdout.toString().trim();
			const data = JSON.parse(output);
			return data.map((b: any) => ({
				name: b.name,
				isCurrent: b.current || false,
				isRemote: b.remote || false,
				ahead: b.ahead || 0,
				behind: b.behind || 0,
			}));
		} catch {
			return this.getGitBranchesFallback();
		}
	}

	/** Create a new virtual branch */
	async createBranch(name: string, base?: string): Promise<string> {
		const args = base
			? [this.cliPath, "branch", "new", name, "--base", base]
			: [this.cliPath, "branch", "new", name];

		const proc = Bun.spawnSync(args, { cwd: this.workingDir });

		if (proc.exitCode !== 0) {
			// Fallback to git checkout -b
			return this.createBranchGitFallback(name, base);
		}

		return proc.stdout.toString().trim() || `Branch '${name}' created`;
	}

	/** Commit changes with a message via GitButler MCP */
	async commit(message: string): Promise<CommitResult> {
		try {
			const client = await this.ensureConnected();

			// Try the MCP commit tool first
			const tools = await client.listTools();
			const commitTool = tools.tools.find(
				(t) =>
					t.name.toLowerCase().includes("commit") ||
					t.name.toLowerCase().includes("save"),
			);

			if (commitTool) {
				const result = await client.callTool({
					name: commitTool.name,
					arguments: { message },
				});

				const text = this.extractText(result);
				return { success: true, hash: this.extractHash(text) };
			}
		} catch (error) {
			console.warn(
				"[GitButler] MCP commit failed, falling back to CLI:",
				error,
			);
		}

		// Fallback to CLI
		return this.commitViaCli(message);
	}

	/** Stage files for commit */
	async stage(files: string[]): Promise<string> {
		const proc = Bun.spawnSync([this.cliPath, "stage", ...files], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			// Fallback to git add
			return this.stageGitFallback(files);
		}

		return proc.stdout.toString().trim() || "Files staged";
	}

	/** Push current stack/branch */
	async push(branch?: string): Promise<string> {
		const args = branch
			? [this.cliPath, "push", branch]
			: [this.cliPath, "push"];

		const proc = Bun.spawnSync(args, { cwd: this.workingDir });

		if (proc.exitCode !== 0) {
			// Fallback to git push
			return this.pushGitFallback(branch);
		}

		return proc.stdout.toString().trim() || "Pushed successfully";
	}

	/** Pull latest changes */
	async pull(): Promise<string> {
		const proc = Bun.spawnSync([this.cliPath, "pull"], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			const err = proc.stderr.toString().trim();
			throw new Error(`Pull failed: ${err}`);
		}

		return proc.stdout.toString().trim() || "Pulled successfully";
	}

	/** Absorb changes into appropriate commits */
	async absorb(): Promise<string> {
		const proc = Bun.spawnSync([this.cliPath, "absorb"], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			const err = proc.stderr.toString().trim();
			throw new Error(`Absorb failed: ${err}`);
		}

		return proc.stdout.toString().trim() || "Changes absorbed";
	}

	/** Discard uncommitted changes */
	async discard(files?: string[]): Promise<string> {
		const args = files
			? [this.cliPath, "discard", ...files]
			: [this.cliPath, "discard"];

		const proc = Bun.spawnSync(args, { cwd: this.workingDir });

		if (proc.exitCode !== 0) {
			// Fallback to git checkout
			return this.discardGitFallback(files);
		}

		return proc.stdout.toString().trim() || "Changes discarded";
	}

	/** Git fallback: discard changes using git checkout */
	private async discardGitFallback(files?: string[]): Promise<string> {
		if (!files || files.length === 0) {
			// Discard all: git checkout -- .
			const proc = Bun.spawnSync(["git", "checkout", "--", "."], {
				cwd: this.workingDir,
			});
			if (proc.exitCode !== 0) {
				throw new Error(
					`Discard (git) failed: ${proc.stderr.toString().trim()}`,
				);
			}
			return "All changes discarded (git fallback)";
		}

		// Discard specific files
		const proc = Bun.spawnSync(["git", "checkout", "--", ...files], {
			cwd: this.workingDir,
		});
		if (proc.exitCode !== 0) {
			throw new Error(`Discard (git) failed: ${proc.stderr.toString().trim()}`);
		}
		return `Discarded ${files.length} file(s) (git fallback)`;
	}

	/** Undo the last commit — files return to unstaged */
	async undoLastCommit(): Promise<{ hash: string; message: string }> {
		// Use git reset --soft HEAD~1 to undo commit but keep changes staged
		const proc = Bun.spawnSync(["git", "reset", "--soft", "HEAD~1"], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			const err = proc.stderr.toString().trim();
			if (
				err.includes("unknown revision") ||
				err.includes("ambiguous argument")
			) {
				throw new Error(
					"No commits to undo — this appears to be the initial commit",
				);
			}
			throw new Error(`Undo failed: ${err}`);
		}

		// Get the hash of the now-current HEAD (the commit before the undone one)
		const hashProc = Bun.spawnSync(["git", "rev-parse", "--short", "HEAD"], {
			cwd: this.workingDir,
		});
		const currentHash = hashProc.stdout.toString().trim();

		// Get the message of the undone commit from reflog
		const reflogProc = Bun.spawnSync(
			["git", "log", "-1", "--format=%s", "HEAD@{1}"],
			{ cwd: this.workingDir },
		);
		const undoneMessage = reflogProc.stdout.toString().trim();

		return {
			hash: currentHash || "unknown",
			message: undoneMessage || "undone commit",
		};
	}

	/** Get the last commit hash + message */
	async getLastCommit(): Promise<{ hash: string; message: string } | null> {
		const proc = Bun.spawnSync(["git", "log", "-1", "--format=%h %s"], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			return null;
		}

		const output = proc.stdout.toString().trim();
		const spaceIdx = output.indexOf(" ");
		if (spaceIdx === -1) return null;

		return {
			hash: output.slice(0, spaceIdx),
			message: output.slice(spaceIdx + 1),
		};
	}

	// --- Fallback methods using raw git ---

	private async getGitStatusFallback(): Promise<GitButlerStatus> {
		const proc = Bun.spawnSync(
			["git", "status", "--porcelain=v2", "--branch"],
			{ cwd: this.workingDir },
		);

		const output = proc.stdout.toString();
		const staged: FileChange[] = [];
		const unstaged: FileChange[] = [];
		const untracked: string[] = [];
		const conflicted: string[] = [];
		let branch = "HEAD";
		let ahead = 0;
		let behind = 0;

		for (const line of output.split("\n")) {
			if (line.startsWith("# branch.head")) {
				branch = line.split(" ").pop() || "HEAD";
			} else if (line.startsWith("# branch.ab")) {
				const parts = line.split(" ");
				ahead = Math.abs(Number(parts.at(-2)));
				behind = Math.abs(Number(parts.at(-1)));
			} else if (line.startsWith("1 ") || line.startsWith("2 ")) {
				const parts = line.split(" ");
				const xy = parts[1];
				const filePath = parts.at(-1)!;

				if (xy[0] !== "." && xy[0] !== "?") {
					staged.push({ path: filePath, status: this.mapStatus(xy[0]) });
				}
				if (xy[1] !== "." && xy[1] !== "?") {
					unstaged.push({ path: filePath, status: this.mapStatus(xy[1]) });
				}
			} else if (line.startsWith("? ")) {
				untracked.push(line.slice(2));
			} else if (line.startsWith("u ")) {
				conflicted.push(line.split(" ").pop()!);
			}
		}

		return { branch, ahead, behind, staged, unstaged, untracked, conflicted };
	}

	private async getGitBranchesFallback(): Promise<BranchInfo[]> {
		const proc = Bun.spawnSync(["git", "branch", "-v", "--no-abbrev"], {
			cwd: this.workingDir,
		});

		const output = proc.stdout.toString();
		return output
			.split("\n")
			.filter((l) => l.trim())
			.map((line) => {
				const isCurrent = line.startsWith("*");
				const name = line.replace(/^\*?\s+/, "").split(/\s+/)[0];
				return { name, isCurrent, isRemote: false, ahead: 0, behind: 0 };
			});
	}

	private async stageGitFallback(files: string[]): Promise<string> {
		const proc = Bun.spawnSync(["git", "add", ...files], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			const err = proc.stderr.toString().trim();
			throw new Error(`Failed to stage files: ${err}`);
		}

		return `Staged ${files.length} file(s) via git add`;
	}

	private async createBranchGitFallback(
		name: string,
		base?: string,
	): Promise<string> {
		const args = base
			? ["git", "checkout", "-b", name, base]
			: ["git", "checkout", "-b", name];

		const proc = Bun.spawnSync(args, { cwd: this.workingDir });

		if (proc.exitCode !== 0) {
			const err = proc.stderr.toString().trim();
			throw new Error(`Failed to create branch: ${err}`);
		}

		return `Branch '${name}' created via git checkout -b`;
	}

	private async pushGitFallback(branch?: string): Promise<string> {
		// Use --set-upstream to handle branches without tracking
		const args = branch
			? ["git", "push", "--set-upstream", "origin", branch]
			: ["git", "push", "--set-upstream", "origin", "HEAD"];

		const proc = Bun.spawnSync(args, { cwd: this.workingDir });

		if (proc.exitCode !== 0) {
			const err = proc.stderr.toString().trim();
			throw new Error(`Push failed: ${err}`);
		}

		return proc.stdout.toString().trim() || "Pushed via git push";
	}

	private async commitViaCli(message: string): Promise<CommitResult> {
		const proc = Bun.spawnSync([this.cliPath, "commit", "-m", message], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			// Fallback to git commit
			return this.commitViaGit(message);
		}

		const output = proc.stdout.toString().trim();
		return { success: true, hash: this.extractHash(output) };
	}

	private async commitViaGit(message: string): Promise<CommitResult> {
		const proc = Bun.spawnSync(["git", "commit", "-m", message], {
			cwd: this.workingDir,
		});

		if (proc.exitCode !== 0) {
			const err = proc.stderr.toString().trim();
			if (
				err.includes("nothing to commit") ||
				err.includes("no changes added to commit")
			) {
				return { success: false, error: "No changes to commit" };
			}
			return { success: false, error: err.replace(/^Error:\s*/, "") };
		}

		const hashProc = Bun.spawnSync(["git", "rev-parse", "--short", "HEAD"], {
			cwd: this.workingDir,
		});
		const hash = hashProc.stdout.toString().trim();
		return { success: true, hash: hash || "unknown" };
	}

	// --- Helpers ---

	private parseButStatus(data: any): GitButlerStatus {
		// Parse GitButler-specific status format
		return {
			branch: data.branch || data.target_branch || "unknown",
			ahead: data.ahead || 0,
			behind: data.behind || 0,
			staged: (data.staged || []).map((f: any) => ({
				path: f.path,
				status: f.status || "modified",
			})),
			unstaged: (data.unstaged || []).map((f: any) => ({
				path: f.path,
				status: f.status || "modified",
			})),
			untracked: data.untracked || [],
			conflicted: data.conflicted || [],
		};
	}

	private mapStatus(code: string): FileChange["status"] {
		switch (code) {
			case "A":
				return "added";
			case "D":
				return "deleted";
			case "R":
				return "renamed";
			default:
				return "modified";
		}
	}

	private extractText(result: any): string {
		if (!result.content) return "";
		return result.content
			.filter((c: any) => c.type === "text")
			.map((c: any) => c.text)
			.join("\n");
	}

	private extractHash(text: string): string {
		const match = text.match(/[0-9a-f]{7,40}/);
		return match ? match[0] : "unknown";
	}
}
