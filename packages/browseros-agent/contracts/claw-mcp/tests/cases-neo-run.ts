/** Verifies SDK forwarding, session tab ownership and grouping, and child cleanup. */
import assert from 'node:assert/strict'
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { describeSteps } from '../../../apps/claw-app/lib/step-sentences'
import { findFreePort } from './browser'
import type { ContractCase } from './cases'
import { apiGet, expectOk, waitUntil } from './helpers'
import { McpSession, readRpcResponse, textOf } from './mcp-client'
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
  {
    name: "neo run: a session's tabs are claimed and grouped",
    async run(ctx) {
      const session = await ctx.openSession('codex')
      await waitUntil(
        async () =>
          !(
            await session.callTool('run', {
              code: 'return context.pages().map(p => p.tabId)',
            })
          ).isError,
        'the SDK browser link',
        { timeoutMs: 30_000 },
      )
      const existingTargets = await pageTargetIds(ctx.browser.cdpPort)
      const failures: unknown[] = []
      try {
        // Use the implicit host task so tasks=[] replies must still project its tabs.
        const opened = await session.callTool('run', {
          code: `const first = await context.newPage();
const second = await context.newPage();
await first.goto(${JSON.stringify(ctx.fixture('/form.html'))});
await second.goto(${JSON.stringify(ctx.fixture('/dynamic.html'))});
return [first.tabId, second.tabId]`,
        })
        expectOk(opened, 'opening two session tabs')
        const ids = opened.structuredContent?.value
        assert(Array.isArray(ids), 'run must return the two native tab ids')
        assert.equal(ids.length, 2)
        const tabIds = ids.map((id: unknown) => {
          assert(typeof id === 'number' && Number.isSafeInteger(id))
          return id
        })
        const firstTabId = tabIds[0]
        assert(firstTabId !== undefined)
        assert.notEqual(firstTabId, tabIds[1])

        const readGroups = async () => {
          const result = await privateBrowserCommand(
            ctx.browser.cdpPort,
            'Browser.getTabGroups',
          )
          assert(result !== null && typeof result === 'object')
          assert('groups' in result && Array.isArray(result.groups))
          return result.groups.map((group: unknown) => {
            assert(group !== null && typeof group === 'object')
            assert('groupId' in group && typeof group.groupId === 'string')
            assert('title' in group && typeof group.title === 'string')
            assert('tabIds' in group && Array.isArray(group.tabIds))
            const members = group.tabIds.map((id: unknown) => {
              assert(typeof id === 'number' && Number.isSafeInteger(id))
              return id
            })
            return {
              groupId: group.groupId,
              title: group.title,
              tabIds: members,
            }
          })
        }
        await waitUntil(async () => {
          const groups = await readGroups()
          const matching = groups.filter((group) =>
            tabIds.some((id) => group.tabIds.includes(id)),
          )
          return (
            matching.length === 1 &&
            matching[0]?.title.startsWith('codex/') === true &&
            tabIds.every((id) => matching[0]?.tabIds.includes(id))
          )
        }, 'both session tabs in one codex/task group')

        await waitUntil(async () => {
          const response = await apiGet(
            ctx.server,
            '/api/v1/sessions?status=live',
          )
          assert(
            response.ok,
            `cockpit session list failed (${response.status})`,
          )
          const result: unknown = await response.json()
          assert(result !== null && typeof result === 'object')
          assert('items' in result && Array.isArray(result.items))
          const own: unknown = result.items.find(
            (item: unknown) =>
              item !== null &&
              typeof item === 'object' &&
              'sessionId' in item &&
              item.sessionId === session.sessionId,
          )
          if (own === undefined) return false
          assert(own !== null && typeof own === 'object')
          assert(
            'live' in own && own.live !== null && typeof own.live === 'object',
          )
          assert(
            'browserTabs' in own.live && Array.isArray(own.live.browserTabs),
          )
          const claimed = own.live.browserTabs.map((tab: unknown) => {
            assert(tab !== null && typeof tab === 'object')
            assert(
              'browserTabId' in tab && typeof tab.browserTabId === 'number',
            )
            if (tabIds.includes(tab.browserTabId)) {
              assert('toolCount' in tab)
              assert.equal(
                tab.toolCount,
                1,
                'one run counted twice for a new tab',
              )
            }
            return tab.browserTabId
          })
          return tabIds.every((id) => claimed.includes(id))
        }, 'the cockpit to list both tabs for the session')

        // A second CDP client represents an agent's explicit placement. Future
        // host reconciliation must leave an already-claimed tab in that group.
        const regrouped = await privateBrowserCommand(
          ctx.browser.cdpPort,
          'Browser.createTabGroup',
          { tabIds: [firstTabId], title: 'agent placement' },
        )
        assert(regrouped !== null && typeof regrouped === 'object')
        assert('group' in regrouped && regrouped.group !== null)
        assert(
          typeof regrouped.group === 'object' && 'groupId' in regrouped.group,
        )
        const explicitGroupId = regrouped.group.groupId
        assert(typeof explicitGroupId === 'string')
        const later = await session.callTool('run', {
          code: 'return context.pages().map(p => p.tabId)',
        })
        expectOk(later, 'a later run after explicit regrouping')
        const groups = await readGroups()
        assert(
          groups.some(
            (group) =>
              group.groupId === explicitGroupId &&
              group.title === 'agent placement' &&
              group.tabIds.includes(firstTabId),
          ),
          'a later run moved or renamed the explicitly grouped tab',
        )
      } catch (error) {
        failures.push(error)
      }
      try {
        if (await ctx.browser.isRunning()) {
          await closeCreatedPages(ctx.browser.cdpPort, existingTargets)
        }
      } catch (error) {
        failures.push(error)
      }
      if (failures.length === 1) throw failures[0]
      if (failures.length > 1) {
        throw new AggregateError(
          failures,
          'Neo grouping case and cleanup failed',
        )
      }
    },
  },
  {
    name: 'neo run: three steps reach the cockpit with redacted values',
    async run(ctx) {
      const session = await ctx.openSession('neo-steps-contract')
      // Resolve the lazy implicit page before counting this run's three calls.
      await waitUntil(
        async () =>
          !(
            await session.callTool('run', { code: 'return await page.title()' })
          ).isError,
        'the SDK browser link',
        { timeoutMs: 30_000 },
      )
      const secret = 'c8-private-typed-value'
      const result = await session.callTool('run', {
        code: `await page.goto(${JSON.stringify(ctx.fixture('/form.html'))});
await page.locator('input').first().fill(${JSON.stringify(secret)});
return await page.title()`,
      })
      expectOk(result, 'a three-step run')
      assert(
        '_meta' in result &&
          result._meta !== null &&
          typeof result._meta === 'object',
      )
      assert('com.browseros.neo/run' in result._meta)
      const sdkRun = result._meta['com.browseros.neo/run']
      assert(
        sdkRun !== null &&
          typeof sdkRun === 'object' &&
          'id' in sdkRun &&
          typeof sdkRun.id === 'string',
      )
      await waitUntil(async () => {
        const response = await apiGet(
          ctx.server,
          `/api/v1/sessions/${session.sessionId}`,
        )
        assert(response.ok)
        const detail: unknown = await response.json()
        assert(detail !== null && typeof detail === 'object')
        assert('session' in detail && detail.session !== null)
        assert(typeof detail.session === 'object')
        assert(
          'tokenUsage' in detail.session,
          'child rows disabled token accounting',
        )
        assert('dispatches' in detail && Array.isArray(detail.dispatches))
        const rows = detail.dispatches.map((row: unknown) => {
          assert(row !== null && typeof row === 'object')
          assert('dispatchKey' in row && typeof row.dispatchKey === 'string')
          assert('toolName' in row && typeof row.toolName === 'string')
          return row
        })
        // Match this run explicitly while waiting for async audit persistence.
        // Every visible child must already have its actual parent in the route.
        for (const row of rows) {
          if (
            'parentDispatchId' in row &&
            typeof row.parentDispatchId === 'string'
          ) {
            assert(
              rows.some(
                (parent) => parent.dispatchKey === row.parentDispatchId,
              ),
              'a child appeared before its parent',
            )
          }
        }
        const parent = rows.find((row) => {
          if (
            row.toolName !== 'run' ||
            !('resultMeta' in row) ||
            typeof row.resultMeta !== 'string'
          )
            return false
          const metadata: unknown = JSON.parse(row.resultMeta)
          return (
            metadata !== null &&
            typeof metadata === 'object' &&
            'sdkRunId' in metadata &&
            metadata.sdkRunId === sdkRun.id
          )
        })
        if (!parent) return false
        const children = rows.filter(
          (row) =>
            'parentDispatchId' in row &&
            row.parentDispatchId === parent.dispatchKey,
        )
        if (children.length < 3) return false
        assert.equal(
          children.length,
          3,
          JSON.stringify(
            rows.map((row) => ({
              tool: row.toolName,
              key: row.dispatchKey,
              parent: 'parentDispatchId' in row ? row.parentDispatchId : null,
            })),
          ),
        )
        assert.deepEqual(children.map((row) => row.toolName).sort(), [
          'input.fill',
          'nav.goto',
          'read',
        ])
        assert(!JSON.stringify([parent, ...children]).includes(secret))
        for (const row of [parent, ...children]) {
          assert('argsJson' in row && typeof row.argsJson === 'string')
          assert(row.argsJson.includes('[redacted]'))
          assert(!('url' in row) || row.url === null)
          assert(!('title' in row) || row.title === null)
        }
        const displayed = describeSteps(
          [parent, ...children].map((row) => {
            assert('dispatchId' in row && typeof row.dispatchId === 'number')
            assert('createdAt' in row && typeof row.createdAt === 'number')
            assert('slug' in row && typeof row.slug === 'string')
            assert('label' in row && typeof row.label === 'string')
            assert('sessionId' in row && typeof row.sessionId === 'string')
            assert('argsJson' in row && typeof row.argsJson === 'string')
            return {
              dispatchId: row.dispatchId,
              createdAt: row.createdAt,
              slug: row.slug,
              label: row.label,
              sessionId: row.sessionId,
              toolName: row.toolName,
              argsJson: row.argsJson,
            }
          }),
        )
        assert.deepEqual(
          displayed.steps.map((step) => step.sentence),
          ['Opened a new page', 'Typed into the page', 'Read the page'],
        )
        return true
      }, 'one run parent and three redacted child steps in the cockpit session')
      expectOk(
        await session.callTool('run', {
          code: `try { await __neo.invoke(${JSON.stringify(secret)}, {}); } catch {} return 1`,
        }),
      )
      await waitUntil(async () => {
        const response = await apiGet(
          ctx.server,
          `/api/v1/sessions/${session.sessionId}`,
        )
        const detail: unknown = await response.json()
        assert(
          !JSON.stringify(detail).includes(secret),
          'unknown operation name escaped redaction',
        )
        assert(detail !== null && typeof detail === 'object')
        assert('dispatches' in detail && Array.isArray(detail.dispatches))
        return detail.dispatches.some(
          (row: unknown) =>
            row !== null &&
            typeof row === 'object' &&
            'toolName' in row &&
            row.toolName === 'sdk.operation',
        )
      }, 'a redacted unknown operation row')
    },
  },
  {
    name: 'neo run: a large screenshot is an MCP image',
    async run(ctx) {
      const session = await ctx.openSession('neo-large-picture')
      await waitUntil(
        async () =>
          !(
            await session.callTool('run', { code: 'return await page.title()' })
          ).isError,
        'the SDK browser link',
        { timeoutMs: 30_000 },
      )
      const result = await session.callTool('run', {
        code: `await page.setViewportSize({width:1000,height:800});
await page.setContent("<canvas id=c width=900 height=700></canvas><script>const g=document.querySelector('#c').getContext('2d');const d=g.createImageData(900,700);let seed=123;for(let i=0;i<d.data.length;i++){seed=(1664525*seed+1013904223)>>>0;d.data[i]=i%4===3?255:seed>>>24;}g.putImageData(d,0,0);</script>");
return await page.screenshot({clip: {x:0,y:0,width:900,height:700}})`,
      })
      expectOk(result, 'large screenshot')
      const image = result.content.find((block) => block.type === 'image')
      assert(image, 'the SDK blob was not returned as an MCP image')
      assert.equal(image.mimeType, 'image/png')
      assert(typeof image.data === 'string')
      const bytes = Buffer.from(image.data, 'base64')
      assert(
        bytes.length >= 700_000,
        `expected at least 700 KB, got ${bytes.length}`,
      )
      assert(
        bytes
          .subarray(0, 8)
          .equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10])),
      )
      assert.equal(
        result.structuredContent?.value,
        `[image at content[${result.content.indexOf(image)}] (image/png)]`,
      )
      assert(
        !result.content.some((block) =>
          block.text?.includes('Blob bytes are not available'),
        ),
      )
      const missing = await session.callTool('run', {
        code: 'return {"$neo":"blob", id:"b99", size:700000}',
      })
      expectOk(missing, 'a missing output blob must not fail the run')
      assert.deepEqual(missing.structuredContent?.value, {
        $neo: 'blob',
        id: 'b99',
        size: 700000,
      })
      assert(
        missing.content.some((block) =>
          block.text?.includes('Blob bytes are not available'),
        ),
      )
    },
  },
  {
    name: 'neo run: writes to foreign tabs carry an advisory notice',
    async run(ctx) {
      const session = await ctx.openSession('neo-ownership-caller')
      const url = ctx.fixture('/form.html')
      await waitUntil(
        async () =>
          !(
            await session.callTool('run', {
              code: `await page.goto(${JSON.stringify(url)}); return 1`,
            })
          ).isError,
        'the SDK browser link',
        { timeoutMs: 30_000 },
      )
      const userUrl = `${url}?user-tab=${crypto.randomUUID()}`
      // Native creation bypasses the SDK's task holder, just as a user tab does.
      const target = await privateBrowserCommand(
        ctx.browser.cdpPort,
        'Target.createTarget',
        { url: userUrl },
      )
      assert(target !== null && typeof target === 'object')
      assert('targetId' in target && typeof target.targetId === 'string')
      try {
        let userTab: number | undefined
        await waitUntil(async () => {
          const result = await session.callTool('run', {
            code: `return context.pages().find(p => p.url() === ${JSON.stringify(userUrl)})?.tabId`,
          })
          if (typeof result.structuredContent?.value !== 'number') return false
          userTab = result.structuredContent.value
          return true
        }, 'the SDK census of the user-created tab')
        assert(userTab !== undefined)
        const userPage = `context.pages().find(p => p.tabId === ${userTab})`
        const read = await session.callTool('run', {
          code: `return await ${userPage}.title()`,
        })
        expectOk(read, 'reading a user tab')
        assert(!textOf(read).includes('not yours'))
        const clicked = await session.callTool('run', {
          code: `const p = ${userPage}; await p.getByRole('button', {name:'Apply',exact:true}).click(); await p.getByRole('button', {name:'Apply',exact:true}).click(); return 1`,
        })
        expectOk(clicked, 'clicking a user tab remains allowed')
        assert(textOf(clicked).includes('tabs that are not yours'))
        assert(textOf(clicked).includes('the user'))
        assert(textOf(clicked).includes('You are allowed to use'))
        assert.equal(
          textOf(clicked).split(`tab ${userTab} belongs`).length - 1,
          1,
        )
        const own = await session.callTool('run', {
          code: "await page.getByRole('button', {name:'Apply',exact:true}).click(); return 1",
        })
        expectOk(own, 'clicking an owned tab')
        assert(!textOf(own).includes('not yours'))
        const other = await ctx.openSession('neo-other-agent')
        const opened = await other.callTool('run', {
          code: `await page.goto(${JSON.stringify(url)}); return page.tabId`,
        })
        expectOk(opened, 'another agent opening its own tab')
        const otherTab = opened.structuredContent?.value
        assert(typeof otherTab === 'number')
        const otherPage = `context.pages().find(p => p.tabId === ${otherTab})`
        const borrowed = await session.callTool('run', {
          code: `await ${otherPage}.getByRole('button', {name:'Apply',exact:true}).click(); return 1`,
        })
        expectOk(borrowed, 'clicking another agent tab remains allowed')
        assert(textOf(borrowed).includes('tabs that are not yours'))
        assert(textOf(borrowed).includes('another agent'))
        const unsent = await session.callTool('run', {
          code: `await ${otherPage}.locator('#missing-c8-element').click({timeout:50})`,
        })
        assert(unsent.isError)
        assert(
          !textOf(unsent).includes('not yours'),
          'an unsent write produced an ownership notice',
        )
      } finally {
        await privateBrowserCommand(ctx.browser.cdpPort, 'Target.closeTarget', {
          targetId: target.targetId,
        })
      }
    },
  },
]

/** Reads the launcher's private browser independently of the server under test. */
async function privateBrowserCommand(
  cdpPort: number,
  method: string,
  params: Record<string, unknown> = {},
): Promise<unknown> {
  const response = await fetch(`http://127.0.0.1:${cdpPort}/json/version`, {
    signal: AbortSignal.timeout(2_000),
  })
  assert(response.ok, `private browser version failed (${response.status})`)
  const version: unknown = await response.json()
  assert(version !== null && typeof version === 'object')
  assert('webSocketDebuggerUrl' in version)
  assert(typeof version.webSocketDebuggerUrl === 'string')
  const socket = new WebSocket(version.webSocketDebuggerUrl)
  let deadline: ReturnType<typeof setTimeout> | undefined
  try {
    return await new Promise<unknown>((resolve, reject) => {
      deadline = setTimeout(
        () => reject(new Error(`CDP ${method} timed out`)),
        10_000,
      )
      socket.addEventListener('open', () => {
        socket.send(JSON.stringify({ id: 1, method, params }))
      })
      socket.addEventListener('error', () =>
        reject(new Error('private CDP socket failed')),
      )
      socket.addEventListener('close', () =>
        reject(new Error('private CDP socket closed')),
      )
      socket.addEventListener('message', (event) => {
        try {
          const message: unknown = JSON.parse(String(event.data))
          assert(message !== null && typeof message === 'object')
          if (!('id' in message) || message.id !== 1) return
          if ('error' in message) {
            reject(
              new Error(
                `CDP ${method} failed ${JSON.stringify(message.error)}`,
              ),
            )
          } else {
            assert('result' in message, `CDP ${method} returned no result`)
            resolve(message.result)
          }
        } catch (error) {
          reject(error)
        }
      })
    })
  } finally {
    clearTimeout(deadline)
    socket.close()
  }
}
