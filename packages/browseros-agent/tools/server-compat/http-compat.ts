// biome-ignore-all lint/suspicious/noUndeclaredEnvVars: Standalone compatibility CLIs are never Turbo-cached.
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
import { agentCases, cases, type RequestCase } from './http-cases'
import { diffBytes, Ids, normalizeHttp } from './wire'

type Json = Record<string, unknown>
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
  for (const request of agentCases(id)) await send(request)

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
