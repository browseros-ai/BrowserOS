/**
 * Boots each implementation for the cross-server suite. The TypeScript
 * server runs in-process: `createServer` with scripted
 * `CanonicalApiDependencies`. The Rust server runs as the compiled
 * `contract-server` example (claw-server-rust), which seeds real app
 * state to the same shape.
 *
 * The fixtures here — `liveSession` / `endedSession`, tab 101 / page 7
 * / target-7, dispatch 1 with its screenshot — must stay in lockstep
 * with the Rust example's `seed()`: the cases assert the same values
 * against both servers.
 */

import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import type { CanonicalApiDependencies } from '../../../apps/claw-server/src/routes/api-v1'
import { createServer } from '../../../apps/claw-server/src/server'
import {
  Configuration,
  DefaultApi,
  type Harness,
  RECORDING_INGEST_MAX_BYTES,
} from '../../../packages/claw-api/src'

export interface ContractServer {
  name: 'rust' | 'typescript'
  baseUrl: string
  api: DefaultApi
  liveSessionId: string
  endedSessionId: string
  screenshotDispatchId: number
  stop(): Promise<void>
}

const liveSession = {
  sessionId: 'session-live',
  slug: 'codex',
  label: 'Codex',
  name: 'Research BrowserClaw',
  startedAt: 100,
  durationMs: 10,
  dispatchCount: 1,
  toolSequence: ['snapshot'],
  status: 'live' as const,
  errorCount: 0,
}

const endedSession = {
  ...liveSession,
  sessionId: 'session-ended',
  name: 'Completed BrowserClaw research',
  status: 'done' as const,
  endedAt: 120,
}

export async function startTypeScriptServer(): Promise<ContractServer> {
  let telemetryConsent = true
  let recordingEvents = ''
  const recordingBatchIds = new Set<string>()
  const connections = new Map<Harness, boolean>()
  const deps: CanonicalApiDependencies = {
    getSystemInfo: () => ({
      product: 'BrowserClaw',
      version: 'contract-test',
      url: 'http://127.0.0.1:0',
      capabilities: {
        recordingIngestVersion: 2,
        recordingIngestMaxBytes: RECORDING_INGEST_MAX_BYTES,
      },
    }),
    getTelemetry: () => ({
      distinctId: 'contract-test',
      enabled: telemetryConsent,
      consent: telemetryConsent,
    }),
    updateTelemetry(consent) {
      telemetryConsent = consent
      return {
        distinctId: 'contract-test',
        enabled: consent,
        consent,
      }
    },
    listSessions: () => ({ items: [liveSession, endedSession] }),
    getSession: (sessionId) =>
      sessionId === liveSession.sessionId
        ? {
            session: liveSession,
            dispatches: [
              {
                dispatchId: 1,
                createdAt: 100,
                slug: 'codex',
                label: 'Codex',
                sessionId,
                toolName: 'snapshot',
                pageId: 7,
                tabId: 101,
                targetId: 'target-7',
                hasScreenshot: true,
              },
            ],
          }
        : null,
    getSessionState: (sessionId) => {
      if (sessionId === liveSession.sessionId) return 'live'
      if (sessionId === endedSession.sessionId) return 'ended'
      return 'missing'
    },
    cancelSession: () => 0,
    getRecording: (sessionId) =>
      sessionId === liveSession.sessionId
        ? {
            hasData: recordingEvents.length > 0,
            complete: true,
            sizeBytes: recordingEvents.length,
            tabs:
              recordingEvents.length > 0
                ? [
                    {
                      tabId: 101,
                      complete: true,
                      firstEventAt: 100,
                      lastEventAt: 402,
                      segments: [
                        {
                          documentId: '018f47a7-1c2b-7def-8123-0123456789ab',
                          targetId: 'target-7',
                          firstEventAt: 100,
                          lastEventAt: 200,
                          sizeBytes: recordingEvents.length,
                          eventCount: recordingEvents
                            .split('\n')
                            .filter(Boolean).length,
                          hasGap: false,
                        },
                      ],
                    },
                  ]
                : [],
          }
        : null,
    downloadRecordingEvents: async (sessionId) =>
      sessionId === liveSession.sessionId ? recordingEvents : null,
    async appendRecordingEvents(_identity, ndjson, batchId) {
      if (recordingBatchIds.has(batchId)) return { accepted: 0 }
      recordingEvents += ndjson
      recordingBatchIds.add(batchId)
      return {
        accepted: ndjson.split('\n').filter((line) => line.trim()).length,
      }
    },
    async appendLegacyRecordingEvents(_sessionId, association, ndjson) {
      if (
        association.tabId !== 101 ||
        association.pageId !== 7 ||
        association.targetId !== 'target-7'
      ) {
        return null
      }
      recordingEvents += ndjson
      return {
        accepted: ndjson.split('\n').filter((line) => line.trim()).length,
      }
    },
    listTabs: () => ({
      items: [
        {
          tabId: 101,
          pageId: 7,
          targetId: 'target-7',
          sessionId: liveSession.sessionId,
          slug: 'codex',
          label: 'Codex',
          url: 'https://browseros.com',
          title: 'BrowserOS',
          status: 'active',
          firstActivityAt: 100,
          lastActivityAt: 110,
          lastToolName: 'snapshot',
          toolCount: 1,
          recentTools: [{ name: 'snapshot', at: 110 }],
          previewCapturedAt: 123,
        },
      ],
    }),
    getTabPreview: (pageId) =>
      pageId === 7 ? { bytes: new Uint8Array([0xff, 0xd8]) } : null,
    getDispatchScreenshot: (dispatchId) =>
      dispatchId === 1 ? { bytes: new Uint8Array([0xff, 0xd8]) } : null,
    async listConnections() {
      return {
        items: Array.from(connections, ([harness, installed]) => ({
          harness,
          installed,
          message: installed ? 'Connected.' : 'Disconnected.',
        })),
      }
    },
    async connectHarness(harness) {
      connections.set(harness, true)
      return { harness, installed: true, message: 'Connected.' }
    },
    async disconnectHarness(harness) {
      connections.set(harness, false)
      return { harness, installed: false, message: 'Disconnected.' }
    },
  }
  const app = createServer({ canonicalApiDependencies: deps })
  const server = Bun.serve({ port: 0, fetch: app.fetch })
  const baseUrl = `http://127.0.0.1:${server.port}`
  return {
    name: 'typescript',
    baseUrl,
    api: new DefaultApi(new Configuration({ basePath: baseUrl })),
    liveSessionId: liveSession.sessionId,
    endedSessionId: endedSession.sessionId,
    screenshotDispatchId: 1,
    async stop() {
      await server.stop(true)
    },
  }
}

export async function startRustServer(): Promise<ContractServer> {
  const root = resolve(import.meta.dir, '../../..')
  const build = Bun.spawnSync({
    cmd: [
      'cargo',
      'build',
      '-p',
      'claw-server-rust',
      '--example',
      'contract-server',
    ],
    cwd: root,
    stdout: 'pipe',
    stderr: 'pipe',
  })
  if (build.exitCode !== 0) {
    throw new Error(build.stderr.toString())
  }

  const portProbe = Bun.serve({ port: 0, fetch: () => new Response() })
  const port = portProbe.port
  await portProbe.stop(true)
  if (port === undefined) throw new Error('failed to allocate a test port')
  const dataDir = await mkdtemp(resolve(tmpdir(), 'claw-contract-rust-'))
  const homeDir = resolve(dataDir, 'home')
  // Exercise harness detection and Codex linking without touching host MCP config.
  await mkdir(resolve(homeDir, '.codex'), { recursive: true })
  const process = Bun.spawn({
    cmd: [
      resolve(root, 'target/debug/examples/contract-server'),
      port.toString(),
      dataDir,
    ],
    cwd: root,
    env: {
      ...globalThis.process.env,
      HOME: homeDir,
      USERPROFILE: homeDir,
      XDG_CONFIG_HOME: resolve(homeDir, '.config'),
      CLAUDE_CONFIG_DIR: homeDir,
      APPDATA: resolve(homeDir, 'AppData', 'Roaming'),
      LOCALAPPDATA: resolve(homeDir, 'AppData', 'Local'),
    },
    stdout: 'pipe',
    stderr: 'pipe',
  })
  const baseUrl = `http://127.0.0.1:${port}`
  await waitUntilReady(baseUrl, process)
  return {
    name: 'rust',
    baseUrl,
    api: new DefaultApi(new Configuration({ basePath: baseUrl })),
    liveSessionId: liveSession.sessionId,
    endedSessionId: endedSession.sessionId,
    screenshotDispatchId: 1,
    async stop() {
      process.kill()
      await process.exited
      await rm(dataDir, { recursive: true, force: true })
    },
  }
}

async function waitUntilReady(
  baseUrl: string,
  process: Bun.Subprocess,
): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (process.exitCode !== null) {
      throw new Error(`Rust contract server exited with ${process.exitCode}`)
    }
    try {
      const response = await fetch(`${baseUrl}/system/health`)
      if (response.ok) return
    } catch {}
    await Bun.sleep(20)
  }
  process.kill()
  throw new Error('Rust contract server did not become ready')
}
