/**
 * Live-browser contracts for cursor snapshot acquisition and frame stitching.
 * These cases exercise public MCP tools against fixture state; scheduler
 * limits and forced completion order remain covered by the Rust unit tests.
 */

import type { CaseContext, ContractCase } from './cases'
import { expectOk, waitUntil } from './helpers'

const CURSOR_STABLE_LABELS = [
  'Cursor candidate 00',
  'Cursor candidate 32',
  'Cursor candidate 63',
] as const

const CURSOR_REASON_LABELS = [
  'Pointer-only target',
  'Assigned onclick target',
  'Editable target ready',
  'Tabindex target',
] as const

interface CursorProbeState {
  candidateCount: number
  activeMarkerNamespaces: number
  distinctMarkerNamespaces: number
  maxActiveMarkerNamespaces: number
  sentinelPresent: boolean
  vanishingCandidatePresent: boolean
}

function snapshotLine(snapshot: string, label: string): string | undefined {
  return snapshot.split('\n').find((line) => line.includes(label))
}

function refFor(snapshot: string, label: string): string {
  const line = snapshotLine(snapshot, label)
  const ref = line?.match(/\[ref=(e\d+)\]/)?.[1]
  if (!ref) {
    throw new Error(
      `no ref found for "${label}" in:\n${snapshot.slice(0, 800)}`,
    )
  }
  return ref
}

function selectedRefs(
  snapshot: string,
  labels: readonly string[],
): Record<string, string> {
  return Object.fromEntries(
    labels.map((label) => [label, refFor(snapshot, label)]),
  )
}

async function evaluateText(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<string> {
  return expectOk(
    await ctx.mcp.callTool('evaluate', { page, code }),
    `evaluate ${code}`,
  )
}

function parseJsonObject<T>(text: string, context: string): T {
  const start = text.indexOf('{')
  const end = text.lastIndexOf('}')
  if (start === -1 || end < start) {
    throw new Error(`${context} did not return JSON: ${text.slice(0, 400)}`)
  }
  try {
    return JSON.parse(text.slice(start, end + 1)) as T
  } catch (error) {
    throw new Error(
      `${context} returned invalid JSON: ${text.slice(start, end + 1)}`,
      { cause: error },
    )
  }
}

async function cursorProbeState(
  ctx: CaseContext,
  page: number,
): Promise<CursorProbeState> {
  const text = await evaluateText(
    ctx,
    page,
    'return JSON.stringify(window.snapshotCursorFixture.state())',
  )
  return parseJsonObject<CursorProbeState>(text, 'cursor fixture state')
}

async function waitForCursorFixture(
  ctx: CaseContext,
  page: number,
): Promise<void> {
  await waitUntil(async () => {
    const state = await cursorProbeState(ctx, page)
    return state.candidateCount === 64
  }, 'the cursor concurrency fixture to be ready')
}

async function requireCleanCursorProbe(
  ctx: CaseContext,
  page: number,
): Promise<CursorProbeState> {
  let state: CursorProbeState | undefined
  await waitUntil(async () => {
    state = await cursorProbeState(ctx, page)
    return (
      state.activeMarkerNamespaces === 0 && !state.vanishingCandidatePresent
    )
  }, 'cursor markers to be cleaned and the vanishing candidate to disappear')
  if (!state?.sentinelPresent) {
    throw new Error(
      `snapshot cleanup removed the page-owned marker: ${JSON.stringify(state)}`,
    )
  }
  return state
}

export const snapshotConcurrencyCases: ContractCase[] = [
  {
    name: 'snapshot concurrency: real cursor batch keeps refs and exclusions',
    async run(ctx) {
      const page = await ctx.openPage(
        ctx.fixture('/snapshot-cursor-concurrency.html'),
      )
      await waitForCursorFixture(ctx, page)

      const snapshot = expectOk(
        await ctx.mcp.callTool('snapshot', { page }),
        'cursor concurrency snapshot',
      )
      selectedRefs(snapshot, CURSOR_STABLE_LABELS)
      selectedRefs(snapshot, CURSOR_REASON_LABELS)

      const zeroSizedLine = snapshotLine(snapshot, 'Zero-sized excluded target')
      if (zeroSizedLine?.includes('[ref=')) {
        throw new Error(
          `zero-sized cursor candidate received a ref: ${zeroSizedLine}`,
        )
      }
      const state = await requireCleanCursorProbe(ctx, page)
      if (state.distinctMarkerNamespaces === 0) {
        throw new Error(
          `fixture did not observe a snapshot marker namespace: ${JSON.stringify(state)}`,
        )
      }
    },
  },
  {
    name: 'snapshot concurrency: cursor refs remain actionable',
    async run(ctx) {
      const page = await ctx.openPage(
        ctx.fixture('/snapshot-cursor-concurrency.html'),
      )
      await waitForCursorFixture(ctx, page)
      const snapshot = expectOk(
        await ctx.mcp.callTool('snapshot', { page }),
        'cursor action snapshot',
      )

      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: refFor(snapshot, 'Pointer-only target'),
        }),
        'click pointer-only cursor ref',
      )
      await waitUntil(
        async () =>
          (
            await evaluateText(
              ctx,
              page,
              'return document.getElementById("cursor-action-result").textContent',
            )
          ).includes('Pointer-only target clicked'),
        'the pointer-only ref click to update its result',
      )

      const editableRef = refFor(snapshot, 'Editable target ready')
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: editableRef,
        }),
        'focus contenteditable cursor ref',
      )
      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'type',
          ref: editableRef,
          text: 'typed through cursor ref',
        }),
        'type through contenteditable cursor ref',
      )
      await waitUntil(
        async () =>
          (
            await evaluateText(
              ctx,
              page,
              'return document.getElementById("editable-target").textContent',
            )
          ).includes('typed through cursor ref'),
        'contenteditable cursor ref to receive text',
      )
      await requireCleanCursorProbe(ctx, page)
    },
  },
]
