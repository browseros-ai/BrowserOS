import { afterEach, describe, expect, test } from 'bun:test'
import { existsSync } from 'node:fs'
import {
  chmod,
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  symlink,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import {
  apiRoutes,
  type ExportResponse,
  type NeoRunRecord,
  textSize,
  type ViewerInfo,
} from '../../shared/api'
import {
  SNAPSHOT,
  SNAPSHOT_WITH_TIP,
  TIP_LINE,
} from '../../web/test-support/replies'
import { RunStore } from '../log/run-store'
import { runResult, sseEvent, startFakeNeo } from '../test-support/fake-neo'
import { FAKE_TOKEN, startFakeNeoHost } from '../test-support/fake-neo-host'
import { TWO_LINES, twoLineRecord } from '../test-support/neo-records'
import { startViewer } from '../viewer-server'
import { TABLE_ROWS } from './export-readme'
import { type ExportSource, writeExport } from './write-export'

const cleanups: Array<() => Promise<unknown>> = []
afterEach(async () => {
  while (cleanups.length > 0) await cleanups.pop()?.()
})

async function tempDir(prefix: string): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), prefix))
  cleanups.push(() => rm(dir, { recursive: true, force: true }))
  return dir
}

const INFO: ViewerInfo = {
  mcpUrl: 'http://127.0.0.1:27361/mcp',
  upstreamUrl: 'http://127.0.0.1:39010/mcp',
  dataDir: '/state',
  startedAt: '2026-11-11T10:00:00.000Z',
  container: {
    name: 'neo-viewer-vp-test',
    image: 'neo-sdk:0.52.1.2-85d411378-arm64',
    up: true,
  },
}

interface Planned {
  code?: string
  text: string
  isError?: boolean
  neo: NeoRunRecord | null
  agentTaskId?: string
}

/** A store whose log holds these runs, each answered 900 ms after it was sent, 5 s apart. */
async function storeWith(dataDir: string, runs: Planned[]): Promise<RunStore> {
  const store = await RunStore.open(dataDir)
  let at = Date.parse('2026-11-11T10:14:02.100Z')
  for (const [index, planned] of runs.entries()) {
    const code = planned.code ?? TWO_LINES
    const id = store.startRun({
      arrival: store.nextArrival(),
      tool: 'run',
      code,
      timeoutMs: null,
      client: { name: 'claude-code' },
      agentTaskId: planned.agentTaskId ?? null,
      sentAt: new Date(at).toISOString(),
      proxyMs: 1,
      request: {
        raw: JSON.stringify({
          jsonrpc: '2.0',
          id: index,
          method: 'tools/call',
        }),
        jsonRpcId: index,
      },
    })
    store.endRun(id, {
      status: planned.isError ? 'error' : 'ok',
      answeredAt: new Date(at + 900).toISOString(),
      reply: {
        httpStatus: 200,
        contentType: 'text/event-stream',
        contentEncoding: null,
        raw: `data: ${JSON.stringify({ text: planned.text })}\n\n`,
        text: planned.text,
        structuredContent: null,
        isError: planned.isError ?? false,
        meta: {
          'com.browseros.neo/run': { id: `neo-${index + 1}`, status: 'ok' },
        },
        rpcError: null,
        notifications: [],
        keepAlives: 0,
      },
    })
    if (planned.neo) store.attachNeoRun(id, planned.neo)
    at += 5_000
  }
  store.toolsListSeen(textSize('runRun code{"type":"object"}'), at, {
    tools: [
      { name: 'run', description: 'Run code', inputSchema: { type: 'object' } },
    ],
  })
  await store.flush()
  return store
}

async function sourceOf(
  store: RunStore,
  info: ViewerInfo = INFO,
): Promise<ExportSource> {
  const { runs, log, toolsList, rawLog } = store.snapshot()
  return {
    instance: 'vp-test',
    info,
    viewerCommit: 'abc123def',
    log,
    runs,
    toolsList,
    rawLog: await rawLog,
  }
}

/** Every relative link in a Markdown file that does not name a file in the export. */
async function brokenLinks(dir: string, file: string): Promise<string[]> {
  const text = await readFile(join(dir, file), 'utf8')
  return [...text.matchAll(/\]\(([^)\s]+)\)/g)]
    .map((match) => match[1] ?? '')
    .filter((link) => !/^[a-z]+:/.test(link))
    .filter((link) => !existsSync(join(dir, dirname(file), link)))
}

const HUGE = `"${'a'.repeat(300_000)}"`
const FAILED_TEXT =
  'script / strict_mode_violation: locator.click: strict mode violation: 2 elements'

describe('an export', () => {
  test('a failed run, a huge reply and a run with no Neo record: every file, linked, bounded, and README.md agrees with session.json', async () => {
    const stateDir = await tempDir('neo-viewer-export-state-')
    const store = await storeWith(stateDir, [
      {
        text: '{"title":"Example"}',
        neo: twoLineRecord(),
        agentTaskId: 'zuzu-7',
      },
      { text: FAILED_TEXT, isError: true, neo: twoLineRecord(true) },
      { code: 'return big', text: HUGE, neo: twoLineRecord() },
      { code: '// no record\nreturn 1', text: '1', neo: null },
    ])
    const root = await tempDir('neo-viewer-export-root-')
    const now = new Date(2026, 10, 11, 22, 45, 1).getTime()
    const written = await writeExport(await sourceOf(store), { root, now })

    expect(written.dir).toBe(join(root, 'vp-test', 'export-20261111-224501'))
    expect(written.runs).toBe(4)
    expect(written.files).toEqual([
      'README.md',
      'session.json',
      'runs/001.md',
      'runs/001.json',
      'runs/001.reply.json',
      'runs/002.md',
      'runs/002.json',
      'runs/002.reply.txt',
      'runs/003.md',
      'runs/003.json',
      'runs/003.reply.json',
      'runs/004.md',
      'runs/004.json',
      'runs/004.reply.json',
      'log.jsonl',
    ])
    for (const file of written.files)
      expect(existsSync(join(written.dir, file))).toBe(true)
    const onDisk = [
      ...(await readdir(written.dir)).filter((name) => name !== 'runs'),
      ...(await readdir(join(written.dir, 'runs'))).map(
        (name) => `runs/${name}`,
      ),
    ].sort()
    expect(onDisk).toEqual([...written.files].sort())

    // README.md's table is session.json's runs.
    const readme = await readFile(join(written.dir, 'README.md'), 'utf8')
    const session = JSON.parse(
      await readFile(join(written.dir, 'session.json'), 'utf8'),
    )
    const rows = readme
      .split('\n')
      .filter((line) => line.startsWith('| ['))
      .map((line) =>
        line.split(' | ').map((part) => part.replace(/^\| |\s*\|$/g, '')),
      )
    expect(rows).toHaveLength(session.runs.length)
    for (const [index, row] of rows.entries()) {
      const run = session.runs[index]
      expect(row[0]).toBe(`[${run.number}](${run.files.markdown})`)
      expect(row[2]).toBe(run.client.name)
      expect(row[3]).toBe(run.agentTaskId ? `${run.agentTaskId} (host)` : '–')
      expect(row[4]).toBe(run.outcome)
      expect(row[7]).toBe(String(run.operationCount ?? '–'))
      expect(row[10]).toBe(`\`${run.firstStatement}\``)
    }
    expect(session.runs.map((run: { outcome: string }) => run.outcome)).toEqual(
      ['ok', 'failed', 'ok', 'ok'],
    )
    expect(session.totals).toMatchObject({
      runs: 4,
      failed: 1,
      methodCalls: 12,
    })
    expect(session.toolsList.reply.tools[0].name).toBe('run')
    expect(session.export).toMatchObject({
      instance: 'vp-test',
      viewer: { commit: 'abc123def' },
      neo: { image: 'neo-sdk:0.52.1.2-85d411378-arm64' },
      screenshots: 'not exported',
    })
    // Summaries only: no code body or reply in session.json.
    expect(JSON.stringify(session)).not.toContain('aaaaaaaaaa')
    expect(session.runs[0]).not.toHaveProperty('code')
    expect(readme).toContain('Screenshots are not exported.')

    // Every link resolves, from README.md and from each run's Markdown.
    for (const file of written.files.filter((name) => name.endsWith('.md')))
      expect(await brokenLinks(written.dir, file)).toEqual([])

    // Sizes stay bounded: the huge reply is whole in its reply file only.
    expect(readme.split('\n').length).toBeLessThan(300)
    for (const file of written.files.filter((name) => name.endsWith('.md')))
      expect(
        (await readFile(join(written.dir, file), 'utf8')).length,
      ).toBeLessThan(12_000)
    expect(
      await readFile(join(written.dir, 'runs/003.reply.json'), 'utf8'),
    ).toBe(HUGE)
    expect(
      (await readFile(join(written.dir, 'session.json'), 'utf8')).length,
    ).toBeLessThan(20_000)

    // The failed run says why; the run with no record says so; the full record is in its JSON.
    const failed = await readFile(join(written.dir, 'runs/002.md'), 'utf8')
    expect(failed).toContain(`## Error\n\n\`\`\`text\n${FAILED_TEXT}\n\`\`\``)
    expect(await readFile(join(written.dir, 'runs/004.md'), 'utf8')).toContain(
      'No record from Neo for this run',
    )
    const full = JSON.parse(
      await readFile(join(written.dir, 'runs/001.json'), 'utf8'),
    )
    expect(full.code).toBe(TWO_LINES)
    expect(full.parsed).toMatchObject({
      value: { title: 'Example' },
      valueIsJson: true,
    })
    expect(full.neo.operations[1]).toMatchObject({
      name: 'page.goto',
      args: { url: 'https://example.com' },
      argsTruncated: false,
    })

    // The raw log, exactly.
    expect(await readFile(join(written.dir, 'log.jsonl'), 'utf8')).toBe(
      await readFile(join(stateDir, 'log.jsonl'), 'utf8'),
    )

    // A second export in the same second gets a folder of its own.
    const again = await writeExport(await sourceOf(store), { root, now })
    expect(again.dir).toBe(`${written.dir}-2`)
  })

  test("run 8, a returned snapshot then Neo's tip: runs/NNN.md decoded, the reply file exact", async () => {
    const dataDir = await tempDir('nv-export-tip-state-')
    const exportRoot = await tempDir('nv-export-tip-')
    const store = await storeWith(dataDir, [
      { text: SNAPSHOT_WITH_TIP, neo: null },
    ])
    const result = await writeExport(await sourceOf(store), {
      root: exportRoot,
      now: Date.parse('2026-11-12T04:39:45Z'),
    })
    const named = (end: string) =>
      join(result.dir, result.files.find((file) => file.endsWith(end)) ?? end)
    expect(result.files.filter((file) => file.includes('.reply.'))).toEqual([
      'runs/001.reply.txt',
    ])
    const markdown = await readFile(named('001.md'), 'utf8')
    const read = markdown.slice(markdown.indexOf('## What the model read'))
    expect(read).toContain('Shown decoded')
    expect(read).toContain(SNAPSHOT.slice(0, 1_000))
    expect(read).not.toContain('\\n- ')
    // The reply file is the exact text, which is not JSON with the tip after it, so .txt.
    expect(await readFile(named('.reply.txt'), 'utf8')).toBe(SNAPSHOT_WITH_TIP)
    const json = JSON.parse(await readFile(named('001.json'), 'utf8'))
    expect(json.parsed.value).toBe(SNAPSHOT)
    expect(json.parsed.notes).toEqual([TIP_LINE])
  })

  test('README.md stays short with many runs; the rest are listed in session.json', async () => {
    const runs = Array.from({ length: TABLE_ROWS + 50 }, (_, index) => ({
      code: `return ${index}`,
      text: String(index),
      neo: null,
    }))
    const store = await storeWith(
      await tempDir('neo-viewer-export-state-'),
      runs,
    )
    const written = await writeExport(await sourceOf(store), {
      root: await tempDir('neo-viewer-export-root-'),
    })
    const readme = await readFile(join(written.dir, 'README.md'), 'utf8')
    expect(readme.split('\n').length).toBeLessThan(300)
    expect(readme).toContain(
      `Runs ${TABLE_ROWS + 1} to ${TABLE_ROWS + 50} are not listed here`,
    )
    expect(written.files).toContain('runs/200.md')
    const session = JSON.parse(
      await readFile(join(written.dir, 'session.json'), 'utf8'),
    )
    expect(session.runs).toHaveLength(TABLE_ROWS + 50)
  })
})

describe('an export stays one coherent, separate thing', () => {
  test('a Clear and a new run during an export change none of its files', async () => {
    const stateDir = await tempDir('neo-viewer-export-state-')
    const store = await storeWith(stateDir, [
      { text: '1', neo: null },
      { text: '2', neo: null },
    ])
    const { runs, log, toolsList, rawLog } = store.snapshot()
    // Between the click and the files: the list is cleared and a new run arrives.
    await store.clear()
    store.startRun({
      arrival: store.nextArrival(),
      tool: 'run',
      code: 'return "after"',
      timeoutMs: null,
      client: { name: 'claude-code' },
      agentTaskId: null,
      sentAt: new Date().toISOString(),
      proxyMs: 0,
      request: { raw: '{}', jsonRpcId: 9 },
    })
    await store.flush()
    const written = await writeExport(
      {
        instance: 'vp-test',
        info: INFO,
        viewerCommit: null,
        log,
        runs,
        toolsList,
        rawLog: await rawLog,
      },
      { root: await tempDir('neo-viewer-export-root-') },
    )
    const session = JSON.parse(
      await readFile(join(written.dir, 'session.json'), 'utf8'),
    )
    const logged = (await readFile(join(written.dir, 'log.jsonl'), 'utf8'))
      .trim()
      .split('\n')
      .map((line) => JSON.parse(line))
    expect(session.runs.map((run: { id: string }) => run.id)).toEqual(
      logged
        .filter((line) => line.type === 'run-start')
        .map((line) => line.run.id),
    )
    expect(logged[0].clearedAt).toBe(session.log.clearedAt)
    expect(JSON.stringify(logged)).not.toContain('after')
  })

  test('an export refuses to be written inside the state folder, or around it', async () => {
    const store = await storeWith(await tempDir('neo-viewer-export-state-'), [
      { text: '1', neo: null },
    ])
    const root = await tempDir('neo-viewer-export-root-')
    for (const dataDir of [
      root,
      join(root, 'vp-test'),
      join(root, 'vp-test', 'deep'),
    ])
      await expect(
        writeExport(await sourceOf(store, { ...INFO, dataDir }), { root }),
      ).rejects.toThrow('overlap')
    expect(await readdir(root)).toEqual([])
  })

  test('an export refuses a shared or linked /tmp/neo-viewer: only a private folder of this user', async () => {
    const store = await storeWith(await tempDir('neo-viewer-export-state-'), [
      { text: '1', neo: null },
    ])
    const base = await tempDir('neo-viewer-export-base-')
    // Writable by others: anyone could swap the export's folder.
    const open = join(base, 'open')
    await mkdir(open)
    await chmod(open, 0o777)
    await expect(
      writeExport(await sourceOf(store), { root: open }),
    ).rejects.toThrow('is not a private folder of this user')
    // A link to a folder elsewhere, as the root or as the instance's folder: refused, nothing written there.
    const elsewhere = join(base, 'elsewhere')
    await mkdir(elsewhere, { mode: 0o700 })
    await symlink(elsewhere, join(base, 'linked-root'))
    await expect(
      writeExport(await sourceOf(store), { root: join(base, 'linked-root') }),
    ).rejects.toThrow('is not a private folder of this user')
    const root = join(base, 'root')
    await mkdir(root, { mode: 0o700 })
    await symlink(elsewhere, join(root, 'vp-test'))
    await expect(writeExport(await sourceOf(store), { root })).rejects.toThrow(
      'is not a private folder of this user',
    )
    expect(await readdir(elsewhere)).toEqual([])
  })

  test('a JSON-RPC error is what the model read: it gets a reply file too', async () => {
    const store = await RunStore.open(await tempDir('neo-viewer-export-state-'))
    const id = store.startRun({
      arrival: store.nextArrival(),
      tool: 'run',
      code: 'return 1',
      timeoutMs: null,
      client: { name: 'claude-code' },
      agentTaskId: null,
      sentAt: new Date().toISOString(),
      proxyMs: 0,
      request: { raw: '{}', jsonRpcId: 1 },
    })
    store.endRun(id, {
      status: 'rpc-error',
      answeredAt: new Date().toISOString(),
      reply: {
        httpStatus: 200,
        contentType: 'application/json',
        contentEncoding: null,
        raw: '{}',
        text: null,
        structuredContent: null,
        isError: null,
        meta: null,
        rpcError: { code: -32602, message: 'Invalid params' },
        notifications: [],
        keepAlives: 0,
      },
    })
    const written = await writeExport(await sourceOf(store), {
      root: await tempDir('neo-viewer-export-root-'),
    })
    expect(written.files).toContain('runs/001.reply.json')
    expect(
      JSON.parse(
        await readFile(join(written.dir, 'runs/001.reply.json'), 'utf8'),
      ),
    ).toEqual({ code: -32602, message: 'Invalid params' })
    expect(await readFile(join(written.dir, 'runs/001.md'), 'utf8')).toContain(
      '## Error\n\nJSON-RPC error -32602: Invalid params',
    )
  })
})

test('POST /api/export: only from the viewer’s own page, and nothing of the host token or an Authorization header in any file', async () => {
  const neo = startFakeNeo(() => ({
    chunks: [
      sseEvent(runResult(1, '{"title":"Example"}', { title: 'Example' })),
    ],
  }))
  cleanups.push(() => neo.stop())
  const host = startFakeNeoHost()
  cleanups.push(() => host.stop())
  host.respond = (request) =>
    request.path.startsWith('/v1/runs/')
      ? Response.json({
          id: '01a1cc18-ed14-78b3-a30d-2cb4ef81d9f4',
          status: 'ok',
          startedAt: 1_000,
          endedAt: 1_400,
          complete: true,
          tabs: [7],
          operations: [
            {
              id: 'x.1',
              name: 'page.goto',
              kind: 'builtin',
              effect: 'write',
              site: { line: 1, column: 7 },
              startedAt: 1_010,
              durationMs: 300,
              outcome: 'ok',
            },
          ],
        })
      : undefined
  const dataDir = await tempDir('neo-viewer-export-api-')
  const tokenFile = join(dataDir, 'host-token')
  await writeFile(tokenFile, FAKE_TOKEN, { mode: 0o600 })
  const root = await tempDir('neo-viewer-export-root-')
  const viewer = await startViewer(
    {
      port: 0,
      upstream: new URL(neo.url),
      dataDir,
      hostApi: { url: new URL(host.url), tokenFile },
      instance: 'vp-api',
    },
    { exportRoot: root },
  )
  cleanups.push(() => viewer.stop())

  // An agent that sends a bearer token of its own: the header must not reach the export either.
  await fetch(viewer.mcpUrl, {
    method: 'POST',
    headers: {
      'content-type': 'application/json',
      accept: 'application/json, text/event-stream',
      authorization: `Bearer ${FAKE_TOKEN}`,
    },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id: 1,
      method: 'tools/call',
      params: {
        name: 'run',
        arguments: { code: "await page.goto('https://example.com')" },
      },
    }),
  }).then((response) => response.text())
  for (let i = 0; i < 100 && viewer.store.list()[0]?.neoDurationMs == null; i++)
    await Bun.sleep(20)
  expect(viewer.store.list()[0]?.neoDurationMs).toBe(400)

  const post = (origin: string) =>
    fetch(`${viewer.url}${apiRoutes.export}`, {
      method: 'POST',
      headers: { origin },
    })
  expect((await post('http://evil.test')).status).toBe(403)
  const response = await post(viewer.url)
  expect(response.status).toBe(200)
  const written = (await response.json()) as ExportResponse
  expect(written.dir.startsWith(join(root, 'vp-api', 'export-'))).toBe(true)
  for (const file of written.files) {
    const text = await readFile(join(written.dir, file), 'utf8')
    expect(text).not.toContain(FAKE_TOKEN)
    expect(text.toLowerCase()).not.toContain('authorization')
    expect(text.toLowerCase()).not.toContain('bearer')
  }
  expect(await readFile(join(written.dir, 'runs/001.md'), 'utf8')).toContain(
    '- L1 page.goto: +10 ms, 300 ms, ok',
  )

  // Clear empties the list; the export stays.
  await fetch(`${viewer.url}${apiRoutes.clear}`, {
    method: 'POST',
    headers: { origin: viewer.url },
  })
  expect(existsSync(join(written.dir, 'README.md'))).toBe(true)
})
