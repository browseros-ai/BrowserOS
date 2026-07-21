/**
 * Boots each implementation for the cross-server suite. The TypeScript
 * server runs in-process: `createServer` with scripted
 * `CanonicalApiDependencies`. The Rust server runs as the compiled
 * `contract-server` example (claw-server-rust), which seeds real app
 * state to the same shape.
 *
 * The fixtures here — two same-profile live sessions, one zero-tab live
 * session, one ended session, browser tab 101, and two session screenshots
 * must stay in lockstep with the Rust example's `seed()`:
 * the cases assert the same values against both servers.
 */

import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import type { CanonicalApiDependencies } from '../../../apps/claw-server/src/routes/api-v1'
import { createServer } from '../../../apps/claw-server/src/server'
import type { Harness } from '../../../packages/claw-api/src'
import { RECORDING_INGEST_MAX_BYTES } from '../../../packages/shared/src/constants/limits'
import { ContractHttpClient } from './http-client'

export interface ContractServer {
  name: 'rust' | 'typescript'
  baseUrl: string
  api: ContractHttpClient
  liveSessionId: string
  secondLiveSessionId: string
  zeroTabLiveSessionId: string
  endedSessionId: string
  screenshotIds: readonly [number, number]
  stop(): Promise<void>
}

const primarySession = {
  sessionId: 'session-live',
  profileId: 'profile-shared',
  slug: 'codex',
  label: 'Codex',
  name: 'Research BrowserClaw',
  harness: 'Codex',
  color: '#7A5AF8',
  startedAt: 100,
  durationMs: 10,
  dispatchCount: 2,
  toolSequence: ['snapshot', 'snapshot'],
  status: 'live' as const,
  errorCount: 0,
  latestScreenshotId: 2,
}

const liveSession = {
  ...primarySession,
  live: {
    state: 'active' as const,
    browserTabs: [
      {
        browserTabId: 101,
        url: 'https://browseros.com',
        title: 'BrowserOS',
        firstActivityAt: 100,
        lastActivityAt: 110,
        lastToolName: 'snapshot',
        toolCount: 1,
        recentTools: [{ name: 'snapshot', at: 110 }],
      },
      {
        browserTabId: 102,
        url: 'https://example.com',
        title: 'Example Domain',
        toolCount: 0,
        recentTools: [],
      },
    ],
  },
}

const secondLiveSession = {
  ...primarySession,
  sessionId: 'session-live-shared-profile',
  name: 'Compare release notes',
  dispatchCount: 1,
  toolSequence: ['read'],
  latestScreenshotId: undefined,
  live: {
    state: 'idle' as const,
    browserTabs: [
      {
        browserTabId: 201,
        url: 'https://browseros.com/releases',
        title: 'BrowserOS Releases',
        firstActivityAt: 105,
        lastActivityAt: 106,
        lastToolName: 'read',
        toolCount: 1,
        recentTools: [{ name: 'read', at: 106 }],
      },
    ],
  },
}

const zeroTabLiveSession = {
  ...primarySession,
  sessionId: 'session-live-empty',
  profileId: 'profile-empty',
  slug: 'claude-code',
  label: 'Claude Code',
  name: 'Waiting for first tool',
  harness: undefined,
  color: undefined,
  dispatchCount: 0,
  toolSequence: [],
  latestScreenshotId: undefined,
  live: { state: 'idle' as const, browserTabs: [] },
}

const endedSession = {
  ...primarySession,
  sessionId: 'session-ended',
  name: 'Completed BrowserClaw research',
  status: 'done' as const,
  endedAt: 120,
  dispatchCount: 1,
  toolSequence: ['snapshot'],
  latestScreenshotId: undefined,
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
    listSessions: (query) =>
      query.status === 'live'
        ? { items: [liveSession, secondLiveSession, zeroTabLiveSession] }
        : { items: [primarySession, endedSession] },
    getSession: (sessionId) =>
      sessionId === primarySession.sessionId
        ? {
            session: primarySession,
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
                screenshotId: 1,
              },
              {
                dispatchId: 2,
                createdAt: 200,
                slug: 'codex',
                label: 'Codex',
                sessionId,
                toolName: 'snapshot',
                pageId: 7,
                tabId: 101,
                targetId: 'target-7',
                screenshotId: 2,
              },
            ],
          }
        : null,
    getSessionState: (sessionId) => {
      if (
        sessionId === primarySession.sessionId ||
        sessionId === secondLiveSession.sessionId ||
        sessionId === zeroTabLiveSession.sessionId
      ) {
        return 'live'
      }
      if (sessionId === endedSession.sessionId) return 'ended'
      return 'missing'
    },
    cancelSession: () => 0,
    getRecording: (sessionId) =>
      sessionId === primarySession.sessionId
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
                          documentId: '33D25F3CF060E81B14070BC356FF1871',
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
      sessionId === primarySession.sessionId ? recordingEvents : null,
    async appendRecordingEvents(_identity, ndjson, batchId) {
      if (recordingBatchIds.has(batchId)) return { accepted: 0 }
      recordingEvents += ndjson
      recordingBatchIds.add(batchId)
      return {
        accepted: ndjson.split('\n').filter((line) => line.trim()).length,
      }
    },
    getSessionPreview: (sessionId) =>
      sessionId === primarySession.sessionId ||
      sessionId === secondLiveSession.sessionId
        ? { bytes: new Uint8Array([0xff, 0xd8]) }
        : null,
    listSessionScreenshots: (sessionId) => {
      if (sessionId === primarySession.sessionId) {
        return {
          items: [
            { screenshotId: 1, capturedAt: 100, toolName: 'snapshot' },
            { screenshotId: 2, capturedAt: 200, toolName: 'snapshot' },
          ],
        }
      }
      return [
        secondLiveSession.sessionId,
        zeroTabLiveSession.sessionId,
        endedSession.sessionId,
      ].includes(sessionId)
        ? { items: [] }
        : null
    },
    getSessionScreenshot: (sessionId, screenshotId) =>
      sessionId === primarySession.sessionId &&
      (screenshotId === 1 || screenshotId === 2)
        ? { bytes: new Uint8Array([0xff, 0xd8]) }
        : null,
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
    api: new ContractHttpClient(baseUrl),
    liveSessionId: primarySession.sessionId,
    secondLiveSessionId: secondLiveSession.sessionId,
    zeroTabLiveSessionId: zeroTabLiveSession.sessionId,
    endedSessionId: endedSession.sessionId,
    screenshotIds: [1, 2],
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
    api: new ContractHttpClient(baseUrl),
    liveSessionId: primarySession.sessionId,
    secondLiveSessionId: secondLiveSession.sessionId,
    zeroTabLiveSessionId: zeroTabLiveSession.sessionId,
    endedSessionId: endedSession.sessionId,
    screenshotIds: [1, 2],
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
