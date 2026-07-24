/**
 * Live-browser contracts for cursor snapshot acquisition and frame stitching.
 * These cases exercise public MCP tools against fixture state; scheduler
 * limits and forced completion order remain covered by the Rust unit tests.
 */

import type { CaseContext, ContractCase } from './cases'
import { expectOk, parsePageId, waitUntil } from './helpers'

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

const FRAME_REF_LABELS = [
  'Parent action ready',
  'Frame A action ready',
  'Frame A cursor ready',
  'Grandchild action ready',
  'Frame B action ready',
  'Frame B cursor ready',
] as const

const FRAME_TEXT_ORDER = [
  'Parent action ready',
  'Frame A action ready',
  'Frame A cursor ready',
  'Grandchild action ready',
  'Frame B action ready',
  'Frame B cursor ready',
] as const

interface CursorProbeState {
  candidateCount: number
  activeMarkerNamespaces: number
  distinctMarkerNamespaces: number
  maxActiveMarkerNamespaces: number
  sentinelPresent: boolean
  vanishingCandidatePresent: boolean
}

interface FrameProbeState {
  frames: string[]
  ready: boolean
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

function requireLabels(snapshot: string, labels: readonly string[]): void {
  const missing = labels.filter((label) => !snapshot.includes(label))
  if (missing.length > 0) {
    throw new Error(
      `snapshot is missing ${missing.join(', ')}:\n${snapshot.slice(0, 1200)}`,
    )
  }
}

function requireSameRefs(
  actual: Record<string, string>,
  expected: Record<string, string>,
  context: string,
): void {
  if (JSON.stringify(actual) !== JSON.stringify(expected)) {
    throw new Error(
      `${context} changed selected refs:\nexpected ${JSON.stringify(expected)}\nactual ${JSON.stringify(actual)}`,
    )
  }
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

function mixedFrameUrl(ctx: CaseContext): string {
  const url = new URL(ctx.fixture('/snapshot-frame-tree.html'))
  const childOrigin = new URL(ctx.fixture2('/'))
  // Ports separate origins, but Chromium isolates by site. A loopback hostname
  // boundary makes frame B an OOPIF while preserving the secondary server.
  childOrigin.hostname =
    url.hostname === 'localhost' ? '127.0.0.1' : 'localhost'
  url.searchParams.set('childOrigin', childOrigin.origin)
  return url.toString()
}

async function frameProbeState(
  ctx: CaseContext,
  page: number,
): Promise<FrameProbeState> {
  const text = await evaluateText(
    ctx,
    page,
    'return JSON.stringify(window.snapshotFrameFixture.state())',
  )
  return parseJsonObject<FrameProbeState>(text, 'frame fixture state')
}

async function waitForFrameSnapshot(
  ctx: CaseContext,
  page: number,
  labels: readonly string[] = FRAME_REF_LABELS,
): Promise<string> {
  let snapshot = ''
  await waitUntil(
    async () => {
      const state = await frameProbeState(ctx, page)
      if (!state.ready) return false
      const result = await ctx.mcp.callTool('snapshot', { page })
      if (result.isError) return false
      snapshot = expectOk(result, 'settled mixed-frame snapshot')
      return labels.every((label) => snapshot.includes(label))
    },
    `the mixed-frame snapshot to contain ${labels.join(', ')}`,
    { timeoutMs: 30_000 },
  )
  return snapshot
}

async function waitForSnapshotLabels(
  ctx: CaseContext,
  page: number,
  labels: readonly string[],
): Promise<string> {
  let snapshot = ''
  await waitUntil(
    async () => {
      const result = await ctx.mcp.callTool('snapshot', { page })
      if (result.isError) return false
      snapshot = expectOk(result, 'snapshot while waiting for labels')
      return labels.every((label) => snapshot.includes(label))
    },
    `the snapshot to contain ${labels.join(', ')}`,
    { timeoutMs: 30_000 },
  )
  return snapshot
}

function requireFrameTextOrder(snapshot: string): void {
  let previousIndex = -1
  for (const label of FRAME_TEXT_ORDER) {
    const index = snapshot.indexOf(label, previousIndex + 1)
    if (index === -1) {
      throw new Error(
        `mixed-frame text is not in DOM stitch order at "${label}":\n${snapshot.slice(0, 1400)}`,
      )
    }
    previousIndex = index
  }
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

async function armVanishingCandidate(
  ctx: CaseContext,
  page: number,
): Promise<void> {
  await evaluateText(
    ctx,
    page,
    'window.snapshotCursorFixture.armVanishingCandidate(); return "armed"',
  )
  await waitUntil(async () => {
    const state = await cursorProbeState(ctx, page)
    return state.vanishingCandidatePresent
  }, 'the vanishing cursor candidate to be armed')
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
      await armVanishingCandidate(ctx, page)

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
      const vanishingLine = snapshotLine(snapshot, 'Vanishing cursor target')
      if (vanishingLine?.includes('[ref=')) {
        throw new Error(
          `vanished cursor candidate received a ref: ${vanishingLine}`,
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
  {
    name: 'snapshot concurrency: mixed frames keep stitch and ref order',
    async run(ctx) {
      const page = await ctx.openPage(mixedFrameUrl(ctx))
      const first = await waitForFrameSnapshot(ctx, page)
      requireFrameTextOrder(first)
      const firstRefs = selectedRefs(first, FRAME_REF_LABELS)

      const frameARef = Number(firstRefs['Frame A action ready'].slice(1))
      const frameBRef = Number(firstRefs['Frame B action ready'].slice(1))
      if (frameBRef >= frameARef) {
        throw new Error(
          `reverse sibling ref allocation changed: frame B=${frameBRef}, frame A=${frameARef}`,
        )
      }

      const second = await waitForFrameSnapshot(ctx, page)
      requireFrameTextOrder(second)
      requireSameRefs(
        selectedRefs(second, FRAME_REF_LABELS),
        firstRefs,
        'second settled mixed-frame snapshot',
      )
    },
  },
  {
    name: 'snapshot concurrency: refs act across every frame session',
    async run(ctx) {
      const page = await ctx.openPage(mixedFrameUrl(ctx))
      const initial = await waitForFrameSnapshot(ctx, page)

      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: refFor(initial, 'Frame A action ready'),
        }),
        'click same-process frame ref',
      )
      const afterFrameA = await waitForSnapshotLabels(ctx, page, [
        'Frame A action clicked',
        'Frame B action ready',
        'Grandchild action ready',
      ])
      requireLabels(afterFrameA, [
        'Parent action ready',
        'Frame B cursor ready',
      ])

      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: refFor(initial, 'Frame B action ready'),
        }),
        'click cross-origin frame ref',
      )
      const afterFrameB = await waitForSnapshotLabels(ctx, page, [
        'Frame A action clicked',
        'Frame B action clicked',
        'Grandchild action ready',
      ])
      requireLabels(afterFrameB, [
        'Parent action ready',
        'Frame A cursor ready',
      ])

      expectOk(
        await ctx.mcp.callTool('act', {
          page,
          kind: 'click',
          ref: refFor(initial, 'Grandchild action ready'),
        }),
        'click nested grandchild frame ref',
      )
      const final = await waitForSnapshotLabels(ctx, page, [
        'Frame A action clicked',
        'Frame B action clicked',
        'Grandchild action clicked',
      ])
      requireLabels(final, [
        'Parent action ready',
        'Frame A cursor ready',
        'Frame B cursor ready',
      ])
    },
  },
  {
    name: 'snapshot concurrency: tabs new returns complete frame auto-context',
    async run(ctx) {
      let page: number | undefined
      try {
        const result = await ctx.mcp.callTool('tabs', {
          action: 'new',
          url: mixedFrameUrl(ctx),
          background: false,
        })
        const text = expectOk(result, 'tabs new mixed-frame auto-context')
        page = parsePageId(result)
        requireLabels(text, FRAME_REF_LABELS)
        selectedRefs(text, FRAME_REF_LABELS)
        requireFrameTextOrder(text)
      } finally {
        if (page !== undefined) {
          await ctx.mcp
            .callTool('tabs', { action: 'close', page })
            .catch(() => {})
        }
      }
    },
  },
  {
    name: 'snapshot concurrency: overlapping cursor captures stay isolated',
    async run(ctx) {
      const page = await ctx.openPage(
        ctx.fixture('/snapshot-cursor-concurrency.html'),
      )
      await waitForCursorFixture(ctx, page)
      await requireCleanCursorProbe(ctx, page)
      await evaluateText(
        ctx,
        page,
        'window.snapshotCursorFixture.resetProbe(); return "reset"',
      )

      const snapshots = await Promise.all(
        Array.from({ length: 4 }, async () =>
          expectOk(
            await ctx.mcp.callTool('snapshot', { page }),
            'overlapping cursor snapshot',
          ),
        ),
      )
      const expectedRefs = selectedRefs(snapshots[0], CURSOR_STABLE_LABELS)
      for (const [index, snapshot] of snapshots.entries()) {
        requireSameRefs(
          selectedRefs(snapshot, CURSOR_STABLE_LABELS),
          expectedRefs,
          `overlapping cursor snapshot ${index + 1}`,
        )
      }

      const overlapState = await requireCleanCursorProbe(ctx, page)
      if (overlapState.maxActiveMarkerNamespaces < 2) {
        throw new Error(
          `cursor captures were serialized instead of overlapping: ${JSON.stringify(overlapState)}`,
        )
      }

      const final = expectOk(
        await ctx.mcp.callTool('snapshot', { page }),
        'final cursor snapshot after overlap',
      )
      requireSameRefs(
        selectedRefs(final, CURSOR_STABLE_LABELS),
        expectedRefs,
        'final cursor snapshot after overlap',
      )
      await requireCleanCursorProbe(ctx, page)
    },
  },
  {
    name: 'snapshot concurrency: overlapping frame captures keep baseline',
    async run(ctx) {
      const page = await ctx.openPage(mixedFrameUrl(ctx))
      const settled = await waitForFrameSnapshot(ctx, page)
      const expectedRefs = selectedRefs(settled, FRAME_REF_LABELS)

      const snapshots = await Promise.all(
        Array.from({ length: 4 }, async () =>
          expectOk(
            await ctx.mcp.callTool('snapshot', { page }),
            'overlapping mixed-frame snapshot',
          ),
        ),
      )
      for (const [index, snapshot] of snapshots.entries()) {
        requireLabels(snapshot, FRAME_REF_LABELS)
        requireSameRefs(
          selectedRefs(snapshot, FRAME_REF_LABELS),
          expectedRefs,
          `overlapping mixed-frame snapshot ${index + 1}`,
        )
      }

      const diffResult = await ctx.mcp.callTool('diff', { page })
      const diffText = expectOk(diffResult, 'diff after overlapping snapshots')
      const structured = diffResult.structuredContent as
        | { changed?: boolean }
        | undefined
      if (
        structured?.changed !== false &&
        !/no change|unchanged/i.test(diffText)
      ) {
        throw new Error(
          `overlapping snapshot commits poisoned the diff baseline: ${diffText.slice(0, 400)}`,
        )
      }

      const final = await waitForFrameSnapshot(ctx, page)
      requireSameRefs(
        selectedRefs(final, FRAME_REF_LABELS),
        expectedRefs,
        'final mixed-frame snapshot after overlap',
      )
    },
  },
]
