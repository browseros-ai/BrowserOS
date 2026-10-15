/** Reuses C1's live conformance suite, then compares a deterministic CRUD/error transcript across runtimes. */
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { assertResponse } from '../../contracts/server-api/tests/support'
import {
  type BootedServer,
  bootServer,
  parseKinds,
  runCli,
  trustedOrigin,
  workspace,
} from './boot'
import { diffBytes, Ids, normalizeHttp } from './wire'

type Json = Record<string, unknown>
interface RequestCase {
  name: string
  path: string
  method?: string
  body?: unknown
  raw?: string
  origin?: string | null
  expected: number
  template?: string
}
const conversationId = '33333333-3333-4333-8333-333333333333'
const missingId = '44444444-4444-4444-8444-444444444444'
const provider = {
  type: 'openai-compatible',
  name: 'Compat fixture',
  modelId: 'plain-text',
  contextWindow: 4096,
  apiKey: 'synthetic-secret',
  createdAt: 1_700_000_000_000,
}
const job = {
  name: 'Compat fixture',
  query: 'Never scheduled',
  scheduleType: 'daily',
  scheduleTime: '09:00',
  enabled: false,
  providerId: 'compat-provider',
  createdAt: 1_700_000_000_000,
}

async function liveContract(
  server: BootedServer,
  output: string,
  label: string,
) {
  const child = Bun.spawn(
    [process.execPath, 'test', 'contracts/server-api/tests/live.test.ts'],
    {
      cwd: workspace,
      env: {
        ...process.env,
        HOME: join(server.root, 'home'),
        BROWSEROS_DIR: join(server.root, 'state'),
        SERVER_API_BASE_URL: server.url,
        SERVER_API_ORIGIN: trustedOrigin,
      },
      stdout: 'pipe',
      stderr: 'pipe',
    },
  )
  const timer = setTimeout(() => child.kill('SIGKILL'), 250_000)
  try {
    const [stdout, stderr, code] = await Promise.all([
      new Response(child.stdout).text(),
      new Response(child.stderr).text(),
      child.exited,
    ])
    await writeFile(join(output, `${label}.contract.log`), stdout + stderr)
    if (code !== 0)
      throw new Error(
        `C1 live contract failed against ${server.kind} (${code}):\n${stdout}${stderr}`,
      )
    console.log(
      `PASS ${label} ${server.kind} C1 live contract (original suite)`,
    )
  } finally {
    clearTimeout(timer)
    if (child.exitCode === null) {
      child.kill()
      await child.exited
    }
  }
}

async function transcript(server: BootedServer) {
  const results: unknown[] = []
  const ids = new Ids()
  const replacements: Array<[string, string]> = [
    [server.url, '<server>'],
    [server.root, '<sandbox>'],
  ]
  const send = async (request: RequestCase): Promise<Json> => {
    const method = request.method ?? 'GET'
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    }
    if (request.origin !== null)
      headers.Origin = request.origin ?? trustedOrigin
    const response = await fetch(`${server.url}${request.path}`, {
      method,
      headers,
      body:
        request.raw ??
        (request.body === undefined ? undefined : JSON.stringify(request.body)),
      signal: AbortSignal.timeout(20_000),
    })
    const text = await response.text()
    let value: unknown
    try {
      value = JSON.parse(text)
    } catch {
      value = text
    }
    if (response.status !== request.expected)
      throw new Error(
        `${request.name}: expected ${request.expected}, got ${response.status}: ${text}`,
      )
    if (request.template)
      assertResponse(
        request.template,
        method.toLowerCase(),
        response.status,
        value,
      )
    const selectedHeaders = Object.fromEntries(
      [
        'content-type',
        'access-control-allow-origin',
        'access-control-allow-methods',
        'access-control-allow-credentials',
        'vary',
      ].map((key) => [key, response.headers.get(key)]),
    )
    results.push({
      name: request.name,
      status: response.status,
      headers: selectedHeaders,
      body: normalizeHttp(value, ids, replacements),
    })
    return value as Json
  }
  const cases: RequestCase[] = [
    {
      name: 'health',
      path: '/system/health',
      expected: 200,
      template: '/system/health',
    },
    {
      name: 'health alias',
      path: '/health',
      expected: 200,
      template: '/health',
    },
    { name: 'status', path: '/status', expected: 200, template: '/status' },
    { name: 'empty providers', path: '/providers', expected: 200 },
    {
      name: 'provider create',
      path: '/providers/compat-provider',
      method: 'PUT',
      body: provider,
      expected: 200,
      template: '/providers/{providerId}',
    },
    {
      name: 'provider read redacts secret',
      path: '/providers/compat-provider',
      expected: 200,
      template: '/providers/{providerId}',
    },
    {
      name: 'provider update',
      path: '/providers/compat-provider',
      method: 'PUT',
      body: { ...provider, name: 'Renamed fixture' },
      expected: 200,
    },
    {
      name: 'provider set default',
      path: '/providers/default',
      method: 'PUT',
      body: { providerId: 'compat-provider' },
      expected: 200,
    },
    {
      name: 'provider read default',
      path: '/providers/default',
      expected: 200,
    },
    {
      name: 'conversation import',
      path: `/conversations/${conversationId}`,
      method: 'PUT',
      body: {
        messages: [
          {
            id: 'fixture-user',
            role: 'user',
            parts: [{ type: 'text', text: 'Fixture' }],
          },
        ],
        lastMessagedAt: 1_700_000_000_000,
      },
      expected: 200,
      template: '/conversations/{conversationId}',
    },
    {
      name: 'conversation read',
      path: `/conversations/${conversationId}`,
      expected: 200,
    },
    {
      name: 'conversation idempotent import',
      path: `/conversations/${conversationId}`,
      method: 'PUT',
      body: { messages: [] },
      expected: 200,
    },
    { name: 'conversation list', path: '/conversations', expected: 200 },
    {
      name: 'job create',
      path: '/scheduled-jobs/compat-job',
      method: 'PUT',
      body: job,
      expected: 200,
      template: '/scheduled-jobs/{jobId}',
    },
    {
      name: 'job update',
      path: '/scheduled-jobs/compat-job',
      method: 'PUT',
      body: { ...job, name: 'Renamed job' },
      expected: 200,
    },
    { name: 'job read', path: '/scheduled-jobs/compat-job', expected: 200 },
    { name: 'job list', path: '/scheduled-jobs', expected: 200 },
    {
      name: 'run create',
      path: '/scheduled-job-runs/compat-run',
      method: 'PUT',
      body: {
        jobId: 'compat-job',
        status: 'completed',
        startedAt: 1_700_000_000_000,
        completedAt: 1_700_000_000_100,
        result: 'Fixture',
      },
      expected: 200,
      template: '/scheduled-job-runs/{runId}',
    },
    { name: 'run read', path: '/scheduled-job-runs/compat-run', expected: 200 },
    { name: 'run list', path: '/scheduled-job-runs', expected: 200 },
  ]
  for (const request of cases) await send(request)
  const created = await send({
    name: 'agent create',
    path: '/agents',
    method: 'POST',
    body: { name: 'Compat agent', type: 'codex' },
    expected: 201,
    template: '/agents',
  })
  const id = (created.agent as Json)?.id
  if (typeof id !== 'string') throw new Error('No created agent id')
  replacements.push([id, '<agent>'])
  for (const request of [
    { name: 'agent read', path: `/agents/${id}`, expected: 200 },
    {
      name: 'agent update',
      path: `/agents/${id}`,
      method: 'PUT',
      body: { name: 'Renamed agent' },
      expected: 200,
    },
    { name: 'agent list', path: '/agents', expected: 200 },
    {
      name: 'malformed JSON',
      path: '/providers/broken',
      method: 'PUT',
      raw: '{',
      expected: 500,
      template: '/providers/{providerId}',
    },
    {
      name: 'missing required fields',
      path: '/providers/broken',
      method: 'PUT',
      body: {},
      expected: 400,
      template: '/providers/{providerId}',
    },
    {
      name: 'bad UUID',
      path: '/conversations/not-a-uuid',
      expected: 400,
      template: '/conversations/{conversationId}',
    },
    { name: 'bad chat UUID', path: '/chat/not-a-uuid/state', expected: 400 },
    {
      name: 'missing conversation',
      path: `/conversations/${missingId}`,
      expected: 404,
    },
    { name: 'missing provider', path: '/providers/absent', expected: 404 },
    { name: 'missing agent', path: `/agents/${missingId}`, expected: 404 },
    { name: 'missing job', path: '/scheduled-jobs/absent', expected: 404 },
    {
      name: 'forbidden origin read',
      path: '/providers',
      origin: 'https://forbidden.invalid',
      expected: 403,
    },
    {
      name: 'forbidden origin write',
      path: '/providers/broken',
      method: 'PUT',
      body: provider,
      origin: 'https://forbidden.invalid',
      expected: 403,
    },
    { name: 'no origin read', path: '/providers', origin: null, expected: 200 },
    {
      name: 'no origin write',
      path: '/providers/broken',
      method: 'PUT',
      body: provider,
      origin: null,
      expected: 403,
    },
    {
      name: 'CORS preflight omits PUT',
      path: '/providers',
      method: 'OPTIONS',
      expected: 204,
    },
    {
      name: 'negative MCP delete',
      path: '/mcp',
      method: 'DELETE',
      expected: 404,
    },
    {
      name: 'agent delete',
      path: `/agents/${id}`,
      method: 'DELETE',
      expected: 200,
    },
    {
      name: 'run delete',
      path: '/scheduled-job-runs/compat-run',
      method: 'DELETE',
      expected: 200,
    },
    {
      name: 'job delete',
      path: '/scheduled-jobs/compat-job',
      method: 'DELETE',
      expected: 200,
    },
    {
      name: 'conversation delete',
      path: `/conversations/${conversationId}`,
      method: 'DELETE',
      expected: 200,
    },
    {
      name: 'provider delete',
      path: '/providers/compat-provider',
      method: 'DELETE',
      expected: 200,
    },
  ])
    await send(request)
  return results
}

export async function main() {
  const { a, b } = parseKinds()
  const output = process.env.SERVER_COMPAT_OUTPUT
    ? resolve(process.env.SERVER_COMPAT_OUTPUT)
    : await mkdtemp(join(tmpdir(), 'server-compat-http-'))
  await mkdir(output, { recursive: true })
  console.log(`Artifacts: ${output}`)
  const transcripts: unknown[][] = []
  for (const [index, kind] of [a, b].entries()) {
    const server = await bootServer(kind)
    try {
      await liveContract(server, output, String(index))
      const result = await transcript(server)
      transcripts.push(result)
      await writeFile(
        join(output, `${index}.http.json`),
        JSON.stringify(result, null, 2),
      )
      console.log(
        `PASS ${index === 0 ? 'A' : 'B'} ${kind} ${result.length} scripted HTTP requests`,
      )
    } finally {
      await server.stop()
    }
  }
  const difference = diffBytes(
    JSON.stringify(transcripts[0], null, 2),
    JSON.stringify(transcripts[1], null, 2),
  )
  if (difference) throw new Error(`HTTP differential failed: ${difference}`)
  console.log(
    `PASS ${a}-vs-${b}: C1 live contract and ${transcripts[0].length} status/header/body comparisons`,
  )
}
if (import.meta.main) await runCli(main)
