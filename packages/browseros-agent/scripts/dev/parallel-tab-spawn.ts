#!/usr/bin/env bun
/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Reproduces the parallel-burst pattern that exposed the homepage
 * rollup flicker: fire N `tabs.new + snapshot` chains in parallel
 * against the cockpit's per-agent MCP route, then poll
 * `/cockpit/tabs/activity` once per second for 30 seconds and print
 * a structured PASS/FAIL summary. The rig is the documented
 * validation gate for any future change to `ACTIVE_WINDOW_MS`, the
 * rollup helper, or the per-agent MCP route.
 *
 * Usage:
 *
 *   bun scripts/dev/parallel-tab-spawn.ts \
 *     --slug=claude-code \
 *     --urls=https://example.com,https://example.org,...
 *
 * Optional flags:
 *
 *   --cockpit-url=http://127.0.0.1:9200/cockpit   default
 *   --origin=chrome-extension://...                default uses the claw-app pinned key
 *   --poll-seconds=30                              total window to observe
 *   --hold-seconds=3                               PASS requires the active count to hold for at least this many consecutive samples at N
 *
 * CI does not run this (needs a live BrowserOS). Reviewers run it
 * by hand when the rollup code or constants change; capture the
 * PASS line into the PR description.
 */

const DEFAULT_COCKPIT_URL = 'http://127.0.0.1:9200/cockpit'
const DEFAULT_ORIGIN = 'chrome-extension://cbjjhiahclaiijedfmgafnkmejjoemga'
const DEFAULT_POLL_SECONDS = 30
const DEFAULT_HOLD_SECONDS = 3

interface CliArgs {
  slug: string
  urls: string[]
  cockpitUrl: string
  origin: string
  pollSeconds: number
  holdSeconds: number
}

function parseArgs(argv: ReadonlyArray<string>): CliArgs {
  const flags = new Map<string, string>()
  for (const arg of argv.slice(2)) {
    const m = arg.match(/^--([^=]+)=(.*)$/)
    if (m) flags.set(m[1], m[2])
  }
  const slug = flags.get('slug')
  const urlsRaw = flags.get('urls')
  if (!slug) throw new Error('--slug=<agent-slug> is required')
  if (!urlsRaw) {
    throw new Error('--urls=<comma-separated-urls> is required (need >= 2)')
  }
  const urls = urlsRaw
    .split(',')
    .map((u) => u.trim())
    .filter((u) => u.length > 0)
  if (urls.length < 2) throw new Error('need at least 2 urls')
  return {
    slug,
    urls,
    cockpitUrl: flags.get('cockpit-url') ?? DEFAULT_COCKPIT_URL,
    origin: flags.get('origin') ?? DEFAULT_ORIGIN,
    pollSeconds: Number(flags.get('poll-seconds') ?? DEFAULT_POLL_SECONDS),
    holdSeconds: Number(flags.get('hold-seconds') ?? DEFAULT_HOLD_SECONDS),
  }
}

interface JsonRpcResult {
  result?: {
    content?: Array<{ type: string; text?: string }>
    isError?: boolean
  }
  error?: { message: string }
}

async function postMcp(
  args: CliArgs,
  id: number,
  method: string,
  params: unknown,
): Promise<JsonRpcResult> {
  const res = await fetch(`${args.cockpitUrl}/mcp/${args.slug}`, {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      Accept: 'application/json, text/event-stream',
      Origin: args.origin,
    },
    body: JSON.stringify({ jsonrpc: '2.0', id, method, params }),
  })
  if (!res.ok) {
    throw new Error(`MCP ${method} returned HTTP ${res.status}`)
  }
  return (await res.json()) as JsonRpcResult
}

function extractPageId(rpc: JsonRpcResult): number | null {
  const text = rpc.result?.content?.[0]?.text
  if (!text) return null
  const m = text.match(/opened page (\d+)/)
  return m ? Number(m[1]) : null
}

interface TabActivityRecord {
  targetId: string
  pageId: number
  url: string
  agentId: string
  toolCount: number
  lastToolName: string
  status: 'active' | 'idle'
}

async function fetchActivity(
  args: CliArgs,
): Promise<{ tabs: TabActivityRecord[] }> {
  const res = await fetch(`${args.cockpitUrl}/tabs/activity`, {
    headers: { Accept: '*/*' },
  })
  if (!res.ok) throw new Error(`activity returned HTTP ${res.status}`)
  return (await res.json()) as { tabs: TabActivityRecord[] }
}

async function fireChain(
  args: CliArgs,
  url: string,
  baseId: number,
): Promise<void> {
  const opened = await postMcp(args, baseId, 'tools/call', {
    name: 'tabs',
    arguments: { action: 'new', url, background: true },
  })
  const pageId = extractPageId(opened)
  if (pageId === null) {
    console.log(`[fire ${baseId}] FAILED to open ${url}`)
    return
  }
  console.log(`[fire ${baseId}] opened ${url} -> page=${pageId}`)
  await postMcp(args, baseId + 1, 'tools/call', {
    name: 'snapshot',
    arguments: { page: pageId },
  })
  console.log(`[fire ${baseId}] snapshot done page=${pageId}`)
}

async function ensureCockpitUp(args: CliArgs): Promise<void> {
  try {
    const res = await fetch(`${args.cockpitUrl}/system/health`, {
      headers: { Accept: '*/*' },
    })
    if (!res.ok) throw new Error(`HTTP ${res.status}`)
    const body = (await res.json()) as { status?: string }
    if (body.status !== 'ok') throw new Error(`status=${body.status}`)
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err)
    throw new Error(
      `cockpit ${args.cockpitUrl}/system/health not reachable: ${msg}`,
    )
  }
}

function formatSnapshot(
  ts: string,
  tabs: ReadonlyArray<TabActivityRecord>,
): string {
  const active = tabs.filter((t) => t.status === 'active').length
  const summary = tabs
    .map((t) => `p${t.pageId}:${t.status[0]}:${t.toolCount}:${t.lastToolName}`)
    .join(' | ')
  return `[${ts}] N=${tabs.length} active=${active} ${summary}`
}

function nowStamp(): string {
  return new Date().toISOString().slice(11, 23)
}

async function main(): Promise<number> {
  const args = parseArgs(process.argv)
  await ensureCockpitUp(args)
  const N = args.urls.length

  console.log(
    `parallel-tab-spawn: slug=${args.slug} N=${N} pollSeconds=${args.pollSeconds} holdSeconds=${args.holdSeconds}`,
  )

  // Initialise the MCP session before firing parallel calls so the
  // per-slug McpManager has done its one-shot setup. Without this
  // the first parallel call pays the init cost and looks slower.
  await postMcp(args, 1, 'initialize', {
    protocolVersion: '2024-11-05',
    capabilities: {},
    clientInfo: { name: 'parallel-tab-spawn', version: '0' },
  })

  // Start the parallel chains in the background while we poll.
  const fireStart = Date.now()
  const fires = args.urls.map((url, i) => fireChain(args, url, 1000 * (i + 1)))

  // Poll the activity endpoint once per second; record per-sample
  // active count so we can decide PASS/FAIL after the window closes.
  const activeCounts: number[] = []
  for (let s = 0; s < args.pollSeconds; s++) {
    const tabs = (await fetchActivity(args)).tabs
    const active = tabs.filter((t) => t.status === 'active').length
    activeCounts.push(active)
    console.log(formatSnapshot(nowStamp(), tabs))
    await Bun.sleep(1000)
  }

  await Promise.all(fires)
  const fireMs = Date.now() - fireStart
  console.log(`all ${N} parallel chains finished in ${fireMs}ms`)

  // PASS criterion: at any point during the poll window the active
  // count reached N AND stayed at N for at least `holdSeconds`
  // consecutive samples without dropping. That captures both the
  // "burst all landed" property and the "stayed stable" property.
  let bestRun = 0
  let currentRun = 0
  for (const c of activeCounts) {
    if (c >= N) {
      currentRun += 1
      if (currentRun > bestRun) bestRun = currentRun
    } else {
      currentRun = 0
    }
  }
  const pass = bestRun >= args.holdSeconds

  console.log(
    pass
      ? `PASS active count reached ${N} and held for ${bestRun}s (>=${args.holdSeconds}s)`
      : `FAIL active count never reached ${N} or did not hold for ${args.holdSeconds}s (best run ${bestRun}s)`,
  )
  return pass ? 0 : 1
}

main()
  .then((code) => process.exit(code))
  .catch((err) => {
    console.error(err instanceof Error ? err.message : String(err))
    process.exit(2)
  })
