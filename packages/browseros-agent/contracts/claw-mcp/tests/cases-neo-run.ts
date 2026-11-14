/** Verifies Playwright forwarding and the SDK child's parent-death life line. */
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { findFreePort } from './browser'
import type { ContractCase } from './cases'
import { expectOk, waitUntil } from './helpers'
import { McpSession, readRpcResponse } from './mcp-client'
import { RUST_BINARY } from './rust-server'

function engineChildren(parentPid: number): number[] {
  const result = Bun.spawnSync({
    cmd: ['pgrep', '-P', String(parentPid), '-f', '__neo-sdk'],
  })
  if (result.exitCode === 1) return []
  if (result.exitCode !== 0) {
    throw new Error(`could not inspect SDK children: ${result.stderr}`)
  }
  return result.stdout
    .toString()
    .trim()
    .split(/\s+/)
    .map(Number)
    .filter((pid) => Number.isSafeInteger(pid) && pid > 0)
}

function processExists(pid: number): boolean {
  const result = Bun.spawnSync({ cmd: ['ps', '-p', String(pid), '-o', 'pid='] })
  return result.exitCode === 0 && result.stdout.toString().trim() !== ''
}

async function pageTargetIds(cdpPort: number): Promise<Set<string>> {
  const response = await fetch(`http://127.0.0.1:${cdpPort}/json/list`, {
    signal: AbortSignal.timeout(2_000),
  })
  if (!response.ok) {
    throw new Error(
      `could not list private browser targets: ${response.status}`,
    )
  }
  const targets: unknown = await response.json()
  if (!Array.isArray(targets)) {
    throw new Error('private browser target list was not an array')
  }
  const ids = new Set<string>()
  targets.forEach((target: unknown) => {
    if (
      target !== null &&
      typeof target === 'object' &&
      'type' in target &&
      target.type === 'page' &&
      'id' in target &&
      typeof target.id === 'string'
    ) {
      ids.add(target.id)
    }
  })
  return ids
}

/** The serial case owns only page targets absent before its extra server started. */
async function closeCreatedPages(
  cdpPort: number,
  existingTargets: ReadonlySet<string>,
): Promise<void> {
  const created = [...(await pageTargetIds(cdpPort))].filter(
    (id) => !existingTargets.has(id),
  )
  for (const id of created) {
    const response = await fetch(
      `http://127.0.0.1:${cdpPort}/json/close/${encodeURIComponent(id)}`,
      { signal: AbortSignal.timeout(2_000) },
    )
    if (!response.ok && response.status !== 404) {
      throw new Error(`could not close case target ${id}: ${response.status}`)
    }
  }
  await waitUntil(
    async () => {
      const remaining = await pageTargetIds(cdpPort)
      return [...remaining].every((id) => existingTargets.has(id))
    },
    'the case-created browser targets to close',
    { timeoutMs: 5_000, intervalMs: 25 },
  )
  const remaining = await pageTargetIds(cdpPort)
  if ([...existingTargets].some((id) => !remaining.has(id))) {
    throw new Error('the case removed a pre-existing browser page target')
  }
}

/** Each modern request has no transport session; only the echoed tool handle joins runs. */
async function statelessRun(
  baseUrl: string,
  id: number,
  args: {
    code: string
    session?: string
    agentName?: string
    taskName?: string
  },
) {
  const response = await fetch(`${baseUrl}/mcp`, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      'mcp-protocol-version': '2026-07-28',
      'mcp-method': 'tools/call',
      'mcp-name': 'run',
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id,
      method: 'tools/call',
      params: {
        name: 'run',
        arguments: args,
        _meta: {
          'io.modelcontextprotocol/protocolVersion': '2026-07-28',
          'io.modelcontextprotocol/clientCapabilities': {},
          'io.modelcontextprotocol/clientInfo': {
            name: 'neo-run-stateless-contract',
            version: '1.0',
          },
        },
      },
    }),
    signal: AbortSignal.timeout(120_000),
  })
  if (!response.ok) {
    throw new Error(
      `stateless run failed: ${response.status} ${await response.text()}`,
    )
  }
  if (response.headers.has('mcp-session-id')) {
    throw new Error('stateless run unexpectedly minted a transport session')
  }
  const message = await readRpcResponse(response, id)
  if (message.error) {
    throw new Error(
      `stateless run RPC failed: ${JSON.stringify(message.error)}`,
    )
  }
  const result = message.result
  if (
    result === null ||
    typeof result !== 'object' ||
    ('isError' in result && result.isError === true) ||
    !('structuredContent' in result)
  ) {
    throw new Error(
      `stateless run returned no successful value: ${JSON.stringify(result)}`,
    )
  }
  const structured = result.structuredContent
  if (
    structured === null ||
    typeof structured !== 'object' ||
    !('value' in structured) ||
    !('session' in structured) ||
    typeof structured.session !== 'string' ||
    structured.session.length === 0
  ) {
    throw new Error(
      `stateless run returned no session handle: ${JSON.stringify(result)}`,
    )
  }
  return { value: structured.value, session: structured.session }
}

export const neoRunCases: ContractCase[] = [
  {
    name: 'neo run: playwright drives a real tab',
    smoke: true,
    async run(ctx) {
      await waitUntil(
        async () =>
          !(
            await ctx.mcp.callTool('run', {
              code: 'return context.pages().map(p => p.tabId)',
            })
          ).isError,
        'the suite SDK to finish its initial tab census',
        { timeoutMs: 30_000 },
      )
      // The suite's own SDK may already hold a blank tab. Preserve every target
      // present after its read-only census, including that blank tab.
      const existingTargets = await pageTargetIds(ctx.browser.cdpPort)
      // A separate parent lets this case exercise SIGKILL without poisoning the
      // suite's server. Its browser is still the launcher's private test browser.
      const root = await mkdtemp(join(tmpdir(), 'claw-mcp-neo-run-'))
      const homeDir = join(root, 'home')
      const resources = join(root, 'resources')
      await mkdir(join(homeDir, '.config'), { recursive: true })
      await mkdir(resources, { recursive: true })
      const serverPort = await findFreePort()
      const sidecar = join(root, 'sidecar.json')
      await writeFile(
        sidecar,
        JSON.stringify({
          ports: { server: serverPort, cdp: ctx.browser.cdpPort },
          directories: { resources },
          flags: { devMode: false },
        }),
      )
      const parent = Bun.spawn({
        cmd: [RUST_BINARY, '--config', sidecar],
        cwd: resolve(import.meta.dir, '../../..'),
        env: {
          ...process.env,
          HOME: homeDir,
          USERPROFILE: homeDir,
          XDG_CONFIG_HOME: join(homeDir, '.config'),
          CLAUDE_CONFIG_DIR: homeDir,
          APPDATA: join(homeDir, 'AppData', 'Roaming'),
          LOCALAPPDATA: join(homeDir, 'AppData', 'Local'),
          BROWSERCLAW_DIR: join(root, 'browserclaw'),
          BROWSEROS_DIR: join(root, 'browseros'),
          BROWSEROS_OUTPUT_DIR: join(root, 'browseros', 'tool-output'),
          BROWSERCLAW_HOSTED_MODE: '0',
          CLAW_ANALYTICS_ENABLED: 'false',
        },
        stdin: 'ignore',
        stdout: 'ignore',
        stderr: 'inherit',
      })
      let enginePid: number | undefined
      const failures: unknown[] = []
      try {
        const baseUrl = `http://127.0.0.1:${serverPort}`
        await waitUntil(
          async () => {
            if (parent.exitCode !== null) {
              throw new Error(`private claw server exited (${parent.exitCode})`)
            }
            try {
              return (
                await fetch(`${baseUrl}/system/health`, {
                  signal: AbortSignal.timeout(1_000),
                })
              ).ok
            } catch {
              return false
            }
          },
          'private claw server health',
          { timeoutMs: 30_000 },
        )
        const mcp = await McpSession.connect(baseUrl, 'neo-run-contract')
        await waitUntil(
          async () =>
            (await mcp.listTools())
              .find((tool) => tool.name === 'run')
              ?.description?.startsWith(
                "Runs JavaScript against the user's browser",
              ) ?? false,
          'the SDK run definition',
          { timeoutMs: 30_000 },
        )
        const url = ctx.fixture('/form.html')
        const first = await mcp.callTool('run', {
          code: `await page.goto(${JSON.stringify(url)}); return await page.title()`,
        })
        expectOk(first, 'Playwright navigation')
        if (first.structuredContent?.value !== 'Form fixture') {
          throw new Error(`unexpected title: ${JSON.stringify(first)}`)
        }
        const second = await mcp.callTool('run', { code: 'return page.url()' })
        expectOk(second, 'the session task keeps its page')
        if (second.structuredContent?.value !== url) {
          throw new Error(
            `the second run lost its page: ${JSON.stringify(second)}`,
          )
        }
        // The SDK refuses unknown arguments. Successful runs with these host
        // fields prove the MCP boundary removes them before forwarding.
        const modernFirst = await statelessRun(baseUrl, 101, {
          code: `await page.goto(${JSON.stringify(url)}); return await page.title()`,
          agentName: 'neo-stateless-agent',
          taskName: 'Stateless Playwright continuity',
        })
        if (modernFirst.value !== 'Form fixture') {
          throw new Error(
            `unexpected stateless title: ${JSON.stringify(modernFirst)}`,
          )
        }
        const modernSecond = await statelessRun(baseUrl, 102, {
          code: 'return page.url()',
          session: modernFirst.session,
        })
        if (
          modernSecond.value !== url ||
          modernSecond.session !== modernFirst.session
        ) {
          throw new Error(
            `stateless run lost its page or handle: ${JSON.stringify(modernSecond)}`,
          )
        }
        const children = engineChildren(parent.pid)
        if (children.length !== 1 || children[0] === undefined) {
          throw new Error(
            `expected one SDK child, found ${children.join(', ')}`,
          )
        }
        enginePid = children[0]
        // Only signal the process handle this case created. The recorded child
        // must disappear through stdin EOF even though SIGKILL skips cleanup.
        parent.kill(9)
        await parent.exited
        const recordedEngine = enginePid
        await waitUntil(
          () => !processExists(recordedEngine),
          `SDK child ${recordedEngine} to exit after parent SIGKILL`,
          { timeoutMs: 2_000, intervalMs: 25 },
        )
      } catch (error) {
        failures.push(error)
      }
      try {
        if (parent.exitCode === null) parent.kill(9)
        await parent.exited
        // A failed life-line assertion may leave our child alive. Clean up only
        // that recorded PID, never another server or the owner's browser.
        if (enginePid !== undefined && processExists(enginePid)) {
          process.kill(enginePid, 'SIGKILL')
          const recordedEngine = enginePid
          await waitUntil(
            () => !processExists(recordedEngine),
            `recorded SDK child ${recordedEngine} to exit during cleanup`,
            { timeoutMs: 2_000, intervalMs: 25 },
          )
        }
      } catch (error) {
        failures.push(error)
      }
      // Run after the parent-death assertion so closing tabs cannot make
      // that lifecycle check pass. Include any SDK blank tab created here.
      try {
        if (await ctx.browser.isRunning()) {
          await closeCreatedPages(ctx.browser.cdpPort, existingTargets)
        }
      } catch (error) {
        failures.push(error)
      }
      try {
        await rm(root, { recursive: true, force: true })
      } catch (error) {
        failures.push(error)
      }
      if (failures.length === 1) throw failures[0]
      if (failures.length > 1) {
        throw new AggregateError(failures, 'Neo run case and cleanup failed')
      }
    },
  },
]
