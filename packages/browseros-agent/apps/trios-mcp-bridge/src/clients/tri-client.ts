/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRI Client — t27 CLI wrapper
 *
 * Wraps the `tri` CLI tool for MCP tool exposure.
 * Commands: test, verdict, status, experience, health, doctor
 */

import { readdir, readFile, stat } from "node:fs/promises";
import { join } from "node:path";

export interface TriRunResult {
	ok: boolean;
	exitCode: number;
	stdout: string;
	stderr: string;
	command: string;
}

export interface TriSpecEditResult {
	ok: boolean;
	reason: string;
	specPath: string;
	testPassed?: boolean;
	testOutput?: string;
}

export interface TriExperienceEntry {
	fileName: string;
	content: string;
	modified: string;
}

export class TriClient {
	private cliPath: string;
	private workingDir: string;

	constructor(cliPath: string, workingDir: string) {
		this.cliPath = cliPath;
		this.workingDir = workingDir;
	}

	/** Run any `tri` command with arguments */
	async run(args: string[]): Promise<TriRunResult> {
		const proc = Bun.spawnSync([this.cliPath, ...args], {
			cwd: this.workingDir,
			stdout: "pipe",
			stderr: "pipe",
		});

		return {
			ok: proc.exitCode === 0,
			exitCode: proc.exitCode,
			stdout: proc.stdout.toString().trim(),
			stderr: proc.stderr.toString().trim(),
			command: `tri ${args.join(" ")}`,
		};
	}

	/** Run `tri test <specPath>` and return structured result */
	async testSpec(specPath: string): Promise<TriRunResult> {
		return this.run(["test", specPath]);
	}

	/** Run `tri verdict` to get current spec status */
	async verdict(): Promise<TriRunResult> {
		return this.run(["verdict"]);
	}

	/** Run `tri status` to get project status */
	async status(): Promise<TriRunResult> {
		return this.run(["status"]);
	}

	/** Run `tri health` to check system health */
	async health(): Promise<TriRunResult> {
		return this.run(["health"]);
	}

	/**
	 * Edit a .t27 spec file and run tests.
	 * 1. Write new content to the spec file
	 * 2. Run `tri test <specPath>`
	 * 3. Return verdict
	 */
	async specEdit(
		specPath: string,
		content: string,
		runTest: boolean = true,
	): Promise<TriSpecEditResult> {
		const fullPath = join(this.workingDir, specPath);

		try {
			// Write the spec file
			await Bun.write(fullPath, content);
		} catch (err) {
			return {
				ok: false,
				reason: `Failed to write spec file: ${err instanceof Error ? err.message : String(err)}`,
				specPath,
			};
		}

		if (!runTest) {
			return {
				ok: true,
				reason: `Spec file written (test skipped).`,
				specPath,
			};
		}

		// Run test
		const result = await this.testSpec(specPath);

		return {
			ok: result.ok,
			reason: result.ok
				? `Spec updated and tests passed.`
				: `Spec updated but tests failed:\n${result.stderr || result.stdout}`,
			specPath,
			testPassed: result.ok,
			testOutput: result.stdout + (result.stderr ? `\n${result.stderr}` : ""),
		};
	}

	/**
	 * Read the last N experience entries from .trinity files.
	 * Scans `.trinity/experience/` directory.
	 */
	async readExperiences(
		count: number = 5,
		trinityDir?: string,
	): Promise<TriExperienceEntry[]> {
		const dir = trinityDir || join(this.workingDir, ".trinity", "experience");

		let files: string[];
		try {
			files = await readdir(dir);
		} catch {
			return [];
		}

		// Filter .trinity files and sort by modification time (newest first)
		const trinityFiles = files.filter((f) => f.endsWith(".trinity"));

		const withStats = await Promise.all(
			trinityFiles.map(async (name) => {
				const filePath = join(dir, name);
				try {
					const s = await stat(filePath);
					return { name, mtime: s.mtime.getTime() };
				} catch {
					return { name, mtime: 0 };
				}
			}),
		);

		withStats.sort((a, b) => b.mtime - a.mtime);

		const selected = withStats.slice(0, count);

		const entries: TriExperienceEntry[] = [];
		for (const { name, mtime } of selected) {
			const filePath = join(dir, name);
			try {
				const content = await readFile(filePath, "utf-8");
				entries.push({
					fileName: name,
					content: content.trim(),
					modified: new Date(mtime).toISOString(),
				});
			} catch {
				// Skip unreadable files
			}
		}

		return entries;
	}

	/** Check if `tri` CLI is available */
	async isAvailable(): Promise<boolean> {
		try {
			const result = await this.run(["--help"]);
			return result.ok || result.exitCode === 0;
		} catch {
			return false;
		}
	}
}
