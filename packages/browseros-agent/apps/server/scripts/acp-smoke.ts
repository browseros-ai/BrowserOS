#!/usr/bin/env bun
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * ACP smoke test — characterizes ACP capabilities against a running OpenClaw
 * gateway. This script is intentionally throwaway: it spawns `openclaw acp`
 * inside the gateway container, exercises every method we plan to depend on,
 * and dumps observed behavior to stdout for the Step 0 spike report.
 *
 * Invocations and findings are documented in:
 *   plans/browseros-ai/BrowserOS/features/2026-04-28-2310-claude-code-acp-implementation-roadmap.md
 *
 * Usage:
 *   bun run apps/server/scripts/acp-smoke.ts [scenario]
 *
 * Scenarios:
 *   all           — run every scenario in sequence (default)
 *   init          — initialize + capabilities
 *   text          — text prompt
 *   image         — text + image prompt (verifies model sees the bytes)
 *   list          — listSessions
 *   resume        — resumeSession against a discovered session
 *   load          — loadSession replay
 *   cancel        — cancel mid-prompt
 */

import { type ChildProcessByStdio, spawn } from 'node:child_process'
import { existsSync, readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join } from 'node:path'
import type { Readable, Writable } from 'node:stream'
import { Readable as NodeReadable, Writable as NodeWritable } from 'node:stream'
import type {
  Client,
  ContentBlock,
  ListSessionsResponse,
  RequestPermissionRequest,
  RequestPermissionResponse,
  SessionNotification,
} from '@agentclientprotocol/sdk'
import * as acp from '@agentclientprotocol/sdk'

// ---------------------------------------------------------------------------
// Config — discovered or overridden via env
// ---------------------------------------------------------------------------

const VM_NAME = process.env.ACP_SMOKE_VM_NAME ?? 'browseros-vm'
const CONTAINER_NAME =
  process.env.ACP_SMOKE_CONTAINER_NAME ??
  'browseros-openclaw-openclaw-gateway-1'
const LIMA_HOME =
  process.env.ACP_SMOKE_LIMA_HOME ?? join(homedir(), '.browseros-dev', 'lima')
const HOST_OPENCLAW_JSON =
  process.env.ACP_SMOKE_OPENCLAW_JSON ??
  join(
    homedir(),
    '.browseros-dev',
    'vm',
    'openclaw',
    '.openclaw',
    'openclaw.json',
  )
const AGENT_ID = process.env.ACP_SMOKE_AGENT_ID ?? 'main'
const GATEWAY_PORT = process.env.ACP_SMOKE_GATEWAY_PORT ?? '18789'

const LIMACTL = process.env.ACP_SMOKE_LIMACTL ?? '/opt/homebrew/bin/limactl'

// ---------------------------------------------------------------------------
// Token resolution
// ---------------------------------------------------------------------------

function readGatewayToken(): string {
  if (!existsSync(HOST_OPENCLAW_JSON)) {
    throw new Error(`openclaw.json not found at ${HOST_OPENCLAW_JSON}`)
  }
  const cfg = JSON.parse(readFileSync(HOST_OPENCLAW_JSON, 'utf-8')) as {
    gateway?: { auth?: { token?: string } }
  }
  const token = cfg.gateway?.auth?.token
  if (!token) {
    throw new Error(`no gateway.auth.token in ${HOST_OPENCLAW_JSON}`)
  }
  return token
}

// ---------------------------------------------------------------------------
// Spawn `openclaw acp` inside the container
// ---------------------------------------------------------------------------

interface SpawnedAcp {
  child: ChildProcessByStdio<Writable, Readable, Readable>
  connection: acp.ClientSideConnection
  events: SessionNotification[]
  permissionsReceived: RequestPermissionRequest[]
  defaultPermissionResponse: RequestPermissionResponse
  init: acp.InitializeResponse
  sessionId: string
}

async function spawnOpenClawAcp(opts: {
  resetSession?: boolean
  sessionKey?: string
}): Promise<SpawnedAcp> {
  const token = readGatewayToken()

  // Pass token via env to the bridge (avoids shell quoting issues).
  // openclaw acp doesn't read OPENCLAW_GATEWAY_TOKEN directly per docs — it
  // wants --token / --token-file. We --token <env-substituted> the token.
  // Since limactl shell runs through ssh, we feed the token as an env var
  // and the container picks it up.
  const sessionKey = opts.sessionKey ?? `agent:${AGENT_ID}:main`

  const acpArgs = [
    'openclaw',
    'acp',
    '--url',
    `ws://127.0.0.1:${GATEWAY_PORT}`,
    '--token',
    token,
    '--session',
    sessionKey,
  ]
  if (opts.resetSession) acpArgs.push('--reset-session')

  // limactl shell <vm> -- nerdctl exec -i <container> -e KEY=VAL ... <cmd...>
  const args = [
    'shell',
    VM_NAME,
    '--',
    'nerdctl',
    'exec',
    '-i',
    '-e',
    'OPENCLAW_HIDE_BANNER=1',
    '-e',
    'OPENCLAW_SUPPRESS_NOTES=1',
    CONTAINER_NAME,
    ...acpArgs,
  ]

  console.log(
    `[spawn] ${LIMACTL} ${args.slice(0, 6).join(' ')} ... [token redacted] ...`,
  )

  const child = spawn(LIMACTL, args, {
    stdio: ['pipe', 'pipe', 'pipe'],
    env: { ...process.env, LIMA_HOME },
  }) as ChildProcessByStdio<Writable, Readable, Readable>

  child.on('exit', (code, signal) => {
    console.log(`[child] exited code=${code} signal=${signal}`)
  })

  const stderrBuf: string[] = []
  child.stderr.on('data', (chunk: Buffer) => {
    const line = chunk.toString('utf-8')
    stderrBuf.push(line)
    // Forward to console for diagnostics; openclaw acp logs to stderr.
    process.stderr.write(`[acp stderr] ${line}`)
  })

  const input = NodeWritable.toWeb(child.stdin)
  const output = NodeReadable.toWeb(child.stdout) as ReadableStream<Uint8Array>
  const stream = acp.ndJsonStream(input, output)

  const events: SessionNotification[] = []
  const permissionsReceived: RequestPermissionRequest[] = []

  // Default: allow_once anything (smoke test only).
  const defaultPermissionResponse: RequestPermissionResponse = {
    outcome: { outcome: 'cancelled' },
  }

  const client: Client = {
    async sessionUpdate(params: SessionNotification) {
      events.push(params)
      const u = params.update
      switch (u.sessionUpdate) {
        case 'agent_message_chunk':
          process.stdout.write(
            // @ts-expect-error - content type narrowing varies by version
            u.content?.text ?? '',
          )
          break
        case 'tool_call':
          // @ts-expect-error - field varies by SDK version
          console.log(`\n[tool_call] ${u.title ?? u.toolCallId}`)
          break
        case 'tool_call_update':
          // @ts-expect-error
          console.log(`[tool_call_update] ${u.toolCallId} status=${u.status}`)
          break
        case 'plan':
          console.log('[plan]', JSON.stringify(u))
          break
        case 'agent_thought_chunk':
          // @ts-expect-error
          console.log(`[thought] ${u.content?.text ?? ''}`)
          break
        case 'available_commands_update':
          // @ts-expect-error
          console.log(
            `[commands]`,
            u.availableCommands?.map((c) => c.name).join(', '),
          )
          break
        case 'current_mode_update':
          // @ts-expect-error
          console.log(`[mode]`, u.currentModeId)
          break
        case 'config_option_update':
          // @ts-expect-error
          console.log(`[config_option]`, u.configOptions)
          break
        case 'session_info_update':
          // @ts-expect-error
          console.log(`[session_info]`, u.title)
          break
        case 'user_message_chunk':
          // @ts-expect-error
          console.log(`[user_replay] ${u.content?.text ?? ''}`)
          break
      }
    },
    async requestPermission(req: RequestPermissionRequest) {
      permissionsReceived.push(req)
      console.log('[permission requested]', JSON.stringify(req, null, 2))
      // Respond allow_once if available, else cancel.
      // @ts-expect-error - field shape varies
      const allow = req.options?.find((o) => o.kind === 'allow_once')
      if (allow) {
        return { outcome: { outcome: 'selected', optionId: allow.optionId } }
      }
      return defaultPermissionResponse
    },
  }

  const connection = new acp.ClientSideConnection((_agent) => client, stream)

  console.log('[init] sending initialize')
  const init = await connection.initialize({
    protocolVersion: acp.PROTOCOL_VERSION,
    clientCapabilities: {},
  })
  console.log('[init] response:')
  console.log(JSON.stringify(init, null, 2))

  console.log('[newSession] cwd=/workspace')
  const newSess = await connection.newSession({
    cwd: '/workspace',
    mcpServers: [],
  })
  console.log('[newSession] response:', JSON.stringify(newSess, null, 2))

  return {
    child,
    connection,
    events,
    permissionsReceived,
    defaultPermissionResponse,
    init,
    sessionId: newSess.sessionId,
  }
}

async function tearDown(s: SpawnedAcp): Promise<void> {
  // SDK 0.19.1 has no closeSession on the connection — we kill the child
  // process. The Gateway considers the session idle after the WS drops.
  s.child.kill('SIGTERM')
  await new Promise<void>((r) => setTimeout(r, 500))
  if (!s.child.killed) s.child.kill('SIGKILL')
}

// ---------------------------------------------------------------------------
// Scenarios
// ---------------------------------------------------------------------------

async function scenarioInit(): Promise<void> {
  console.log('\n=== scenario: init ===')
  const s = await spawnOpenClawAcp({})
  console.log('\n[init summary]')
  console.log('  protocolVersion:', s.init.protocolVersion)
  console.log('  agentInfo:', s.init.agentInfo)
  console.log(
    '  promptCapabilities:',
    s.init.agentCapabilities?.promptCapabilities,
  )
  console.log('  loadSession:', s.init.agentCapabilities?.loadSession)
  // @ts-expect-error - field name varies between SDK builds
  console.log(
    '  sessionCapabilities:',
    s.init.agentCapabilities?.sessionCapabilities,
  )
  console.log('  authMethods:', s.init.authMethods)
  await tearDown(s)
}

async function scenarioText(): Promise<void> {
  console.log('\n=== scenario: text prompt ===')
  const s = await spawnOpenClawAcp({})
  console.log('[prompt] sending text...')
  const prompt: ContentBlock[] = [
    { type: 'text', text: 'Reply with just the word "OK" — nothing else.' },
  ]
  // @ts-expect-error - prompt field shape may vary
  const result = await s.connection.prompt({ sessionId: s.sessionId, prompt })
  console.log('\n[prompt] result:', JSON.stringify(result, null, 2))
  console.log(`[events received] count=${s.events.length}`)
  await tearDown(s)
}

async function scenarioImage(): Promise<void> {
  console.log('\n=== scenario: image prompt ===')
  // 1x1 red PNG
  const redPixel =
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg=='
  const s = await spawnOpenClawAcp({})
  if (!s.init.agentCapabilities?.promptCapabilities?.image) {
    console.log(
      '[image] capability not advertised; sending anyway to observe behavior',
    )
  }
  const prompt: ContentBlock[] = [
    {
      type: 'text',
      text:
        'I sent you an image attached to this message. ' +
        'In one short sentence, describe what color the image is. ' +
        'If you cannot see any image, reply exactly: NO_IMAGE_RECEIVED.',
    },
    // @ts-expect-error - ContentBlock image discriminant
    { type: 'image', data: redPixel, mimeType: 'image/png' },
  ]
  // @ts-expect-error
  const result = await s.connection.prompt({ sessionId: s.sessionId, prompt })
  console.log('\n[prompt] result:', JSON.stringify(result, null, 2))
  await tearDown(s)
}

async function scenarioList(): Promise<ListSessionsResponse | null> {
  console.log('\n=== scenario: listSessions ===')
  const s = await spawnOpenClawAcp({})
  let result: ListSessionsResponse | null = null
  try {
    // @ts-expect-error - input shape varies
    result = await s.connection.listSessions({})
    console.log('[listSessions] response (first 5 sessions):')
    // @ts-expect-error
    const first = (result.sessions ?? []).slice(0, 5)
    console.log(JSON.stringify(first, null, 2))
    // @ts-expect-error
    console.log(
      `[listSessions] total=${result.sessions?.length} cursor=${result.nextCursor}`,
    )
  } catch (err) {
    console.log('[listSessions] error:', (err as Error).message)
  }
  await tearDown(s)
  return result
}

async function scenarioResume(): Promise<void> {
  console.log('\n=== scenario: resumeSession ===')
  console.log(
    '[resume] SKIPPED — SDK 0.19.1 does not expose resumeSession;\n' +
      '          OpenClaw advertises sessionCapabilities = {list:{}} only.\n' +
      '          We use loadSession for rebind+replay instead.',
  )
}

async function scenarioLoad(): Promise<void> {
  console.log('\n=== scenario: loadSession ===')
  // session/list is not implemented on this OpenClaw build, so we can't
  // discover existing sessions through ACP. Instead we use the gateway
  // session key the bridge maps to from --session: the freshly-spawned
  // session shares the same `agent:<id>:main` key. We can call loadSession
  // against the sessionId returned by newSession to test the replay path
  // on whatever transcript the gateway has under that key.
  const s = await spawnOpenClawAcp({})
  console.log('[load] calling loadSession against own sessionId...')
  console.log('[load] (this is the only mode we can test without session/list)')
  try {
    // @ts-expect-error - input shape varies
    await s.connection.loadSession({
      sessionId: s.sessionId,
      cwd: '/workspace',
      mcpServers: [],
    })
    console.log('[load] complete')
    console.log(`[load] events received: ${s.events.length}`)
    const counts = s.events.reduce<Record<string, number>>((acc, ev) => {
      const k = ev.update.sessionUpdate
      acc[k] = (acc[k] ?? 0) + 1
      return acc
    }, {})
    console.log('[load] event breakdown:', counts)
    if (s.events.length > 0) {
      console.log('[load] first 2 events:')
      for (const ev of s.events.slice(0, 2)) {
        console.log(JSON.stringify(ev, null, 2))
      }
    }
  } catch (err) {
    console.log('[load] error:', (err as Error).message)
  }
  await tearDown(s)
}

async function scenarioCancel(): Promise<void> {
  console.log('\n=== scenario: cancel ===')
  const s = await spawnOpenClawAcp({})
  const prompt: ContentBlock[] = [
    {
      type: 'text',
      text: 'Count from 1 to 100, one number per line, slowly, taking a fraction of a second between each.',
    },
  ]
  console.log('[cancel] starting prompt...')
  // @ts-expect-error
  const promptPromise = s.connection.prompt({ sessionId: s.sessionId, prompt })
  await new Promise<void>((r) => setTimeout(r, 1500))
  console.log('\n[cancel] sending cancel notification...')
  await s.connection.cancel({ sessionId: s.sessionId })
  const result = await promptPromise
  console.log('\n[cancel] prompt resolved:', JSON.stringify(result, null, 2))
  console.log(
    `[cancel] events before cancel: ${s.events.length}; expected stopReason: cancelled`,
  )
  await tearDown(s)
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

async function main() {
  const arg = process.argv[2] ?? 'all'
  console.log(`[acp-smoke] running scenario: ${arg}`)
  console.log(
    `[acp-smoke] vm=${VM_NAME} container=${CONTAINER_NAME} agent=${AGENT_ID}`,
  )

  try {
    switch (arg) {
      case 'init':
        await scenarioInit()
        break
      case 'text':
        await scenarioText()
        break
      case 'image':
        await scenarioImage()
        break
      case 'list':
        await scenarioList()
        break
      case 'resume':
        await scenarioResume()
        break
      case 'load':
        await scenarioLoad()
        break
      case 'cancel':
        await scenarioCancel()
        break
      case 'all':
        await scenarioInit()
        await scenarioText()
        await scenarioImage()
        await scenarioList()
        await scenarioResume()
        await scenarioLoad()
        await scenarioCancel()
        break
      default:
        console.error(`unknown scenario: ${arg}`)
        process.exit(1)
    }
  } catch (err) {
    console.error('[acp-smoke] FATAL:', err)
    process.exit(1)
  }
}

main().then(() => process.exit(0))
