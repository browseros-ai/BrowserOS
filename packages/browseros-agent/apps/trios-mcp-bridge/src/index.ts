import { StreamableHTTPTransport } from "@hono/mcp";
import { Hono } from "hono";
import { cors } from "hono/cors";
import { createBridgeServer } from "./bridge-server.js";
import { BrowserOSClient } from "./clients/browseros-client.js";
import { GitButlerMcpClient } from "./clients/gitbutler-client.js";
import { TriClient } from "./clients/tri-client.js";
import { type BridgeConfig, loadConfig } from "./config.js";

// Parse CLI args
function parseArgs(): Partial<BridgeConfig> {
	const args = process.argv.slice(2);
	const config: Partial<BridgeConfig> = {};

	for (let i = 0; i < args.length; i++) {
		switch (args[i]) {
			case "--port":
				config.port = Number(args[++i]);
				break;
			case "--browseros-url":
				config.browserosMcpUrl = args[++i];
				break;
			case "--gitbutler-cli":
				config.gitbutlerCliPath = args[++i];
				break;
			case "--tri-cli":
				config.triCliPath = args[++i];
				break;
			case "--working-dir":
				config.workingDir = args[++i];
				break;
			case "--no-internal":
				config.gitbutlerInternal = false;
				break;
			case "--log-level":
				config.logLevel = args[++i];
				break;
			case "--help":
				console.log(`
TRIOS MCP Bridge — Vision + GitButler + t27 CLI

Usage: bun run src/index.ts [options]

Options:
  --port <number>            Bridge server port (default: 9200)
  --browseros-url <url>      BrowserOS MCP URL (default: http://127.0.0.1:9105/mcp)
  --gitbutler-cli <path>       GitButler CLI path (default: but)
  --tri-cli <path>             t27 CLI path (default: tri)
  --working-dir <path>          Working directory for git (default: cwd)
  --no-internal                  Disable GitButler internal MCP tools
  --log-level <level>           Log level: debug|info|warn|error (default: info)

Examples:
  bun run src/index.ts --port 9200
  bun run src/index.ts --browseros-url http://127.0.0.1:9000/mcp
`);
				process.exit(0);
				break;
			default:
		}
	}

	return config;
}

async function _main() {
	const config = loadConfig(parseArgs());

	console.log("═".repeat(60));
	console.log("  TRIOS MCP Bridge — Vision + GitButler + t27 CLI");
	console.log("═".repeat(60));

	// Initialize clients
	const browseros = new BrowserOSClient(config.browserosMcpUrl);
	const gitbutler = new GitButlerMcpClient(
		config.gitbutlerCliPath,
		config.gitbutlerInternal,
		config.workingDir,
	);
	const tri = new TriClient(config.triCliPath, config.workingDir);

	// Create bridge server with deps
	const bridgeDeps = { config, browseros, gitbutler, tri };
	const _server = createBridgeServer(bridgeDeps);

	console.log("  Port:         ${config.port}");
	console.log(`  BrowserOS:    ${config.browserosMcpUrl}`);
	console.log(
		`  GitButler:    ${config.gitbutlerCliPath} (internal: ${config.gitbutlerInternal})`,
	);
	console.log(`  t27 CLI:       ${config.triCliPath}`);
	console.log(`  Working Dir:  ${config.workingDir}`);

	// Check tri availability (warning only)
	tri.isAvailable().then((available) => {
		if (available) {
			console.log("✅ t27 CLI (tri) available");
		} else {
			console.warn(
				"⚠️  t27 CLI (tri) not found — GitButler tools will use CLI fallback",
			);
		}
	});

	// Try initial connections (non-blocking — will retry on first tool call)
	console.log("\n📡 Connecting to BrowserOS MCP...");
	await browseros.connect().catch((err) => {
		console.warn(`⚠️  BrowserOS not available yet: ${err}`);
		console.warn("   Will retry on first tool call.");
	});
	console.log("\n📡 Connecting to GitButler MCP...");
	await gitbutler.connect().catch((err) => {
		console.warn(`⚠️  GitButler MCP not available yet: ${err}`);
		console.warn("   Will use CLI fallback for gitbutler tools.");
	});

	// Set up HTTP server with Hono
	const app = new Hono();

	// CORS for local development
	app.use(
		"*",
		cors({
			origin: "*",
			allowMethods: ["GET", "POST", "OPTIONS", "DELETE"],
			allowHeaders: ["Content-Type"],
		}),
	);

	// Health check endpoint
	app.get("/", (c) => {
		return c.json({
			name: "trios-mcp-bridge",
			version: "0.2.0",
			status: "running",
			connections: {
				browseros: browseros.isConnected ? "connected" : "disconnected",
				gitbutler: gitbutler.isConnected ? "connected" : "disconnected",
			},
			config: {
				port: config.port,
				browserosMcpUrl: config.browserosMcpUrl,
				gitbutlerCliPath: config.gitbutlerCliPath,
				triCliPath: config.triCliPath,
				workingDir: config.workingDir,
				gitbutlerInternal: config.gitbutlerInternal,
				logLevel: config.logLevel,
			},
		});
	});

	// MCP status endpoint (GET /mcp)
	app.get("/mcp", async (c) => {
		return c.json({
			name: "trios-mcp-bridge",
			status: "running",
			tools: 17,
		});
	});

	// MCP request endpoint (POST /mcp) — per-request server + transport
	app.post("/mcp", async (c) => {
		const mcpServer = createBridgeServer({
			config,
			browseros,
			gitbutler,
			tri,
		});

		const transport = new StreamableHTTPTransport({
			sessionIdGenerator: undefined,
			enableJsonResponse: true,
		});

		await mcpServer.connect(transport);

		return transport.handleRequest(c);
	});

	console.log(
		`\n📡 TRIOS MCP Bridge running at http://127.0.0.1:${config.port}/mcp`,
	);
	console.log(`   MCP endpoint:  http://127.0.0.1:${config.port}/mcp`);
	console.log("\n   Press Ctrl+C to stop.\n");

	Bun.serve({
		port: config.port,
		fetch: app.fetch,
	});
}
