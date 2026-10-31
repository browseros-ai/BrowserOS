/**
 * Live-browser contracts for refs inside iframes, through the public snapshot, act and run
 * tools. `frame-lab.html` nests same-site, cross-site and sandboxed frames, the last two as
 * OOPIFs; its targets report each click's `event.isTrusted`, so a pass means native input
 * reached the frame, not a scripted `click()`.
 *
 * Every case runs twice. Unbound, on the primary server, the way a browser without profile
 * targeting runs. Scoped, on a server told the browser's BrowserOS version, the way BrowserOS
 * launches it: its bound views run the frame admission guards. The scoped cases are skipped,
 * and reported as skipped, on a browser without profile targeting.
 */

import type { CaseContext, ContractCase } from './cases'
import { expectError, expectOk, waitUntil } from './helpers'
import type { McpSession } from './mcp-client'

type Mode = 'unbound' | 'scoped'

/** The case context plus the session every call in one case goes through. */
interface Lab {
  ctx: CaseContext
  mcp: McpSession
}

interface LabCase {
  id: string
  /** The target document's frame level; 0 is the top page. */
  level: number
  /**
   * The target document's site: A is the top page's host, B the other loopback name, S is A
   * in a sandboxed iframe.
   */
  site: 'A' | 'B' | 'S'
}

interface LabEvent {
  control: 'button' | 'checkbox'
  trusted: boolean
  origin: string
  level: number
  docId: string
  checked: boolean
}

interface LabState {
  ready: boolean
  inView: boolean
  targetDoc?: string
  events: LabEvent[]
  /** `level:action` for each command a document applied to its own child frame. */
  done: string[]
}

const LAB_CASES: LabCase[] = [
  { id: 'top', level: 0, site: 'A' },
  { id: 'single-same', level: 1, site: 'A' },
  { id: 'nested-same', level: 2, site: 'A' },
  { id: 'single-cross', level: 1, site: 'B' },
  { id: 'nested-cross', level: 2, site: 'B' },
  { id: 'nested-mixed', level: 2, site: 'A' },
  { id: 'sandboxed', level: 1, site: 'S' },
]

async function labIn(ctx: CaseContext, mode: Mode): Promise<Lab> {
  return { ctx, mcp: mode === 'scoped' ? await ctx.scopedMcp() : ctx.mcp }
}

function labCase(id: string): LabCase {
  const found = LAB_CASES.find((candidate) => candidate.id === id)
  if (!found) throw new Error(`unknown frame lab case ${id}`)
  return found
}

function labUrl(lab: Lab, id: string, scroll = false): string {
  const url = new URL(lab.ctx.fixture('/frame-lab.html'))
  url.searchParams.set('case', id)
  if (scroll) url.searchParams.set('scroll', '1')
  return url.toString()
}

/** The origin a case's target document is served from. */
function targetOrigin(lab: Lab, labCase: LabCase): string {
  const origin = new URL(lab.ctx.fixture('/'))
  if (labCase.site === 'B') {
    origin.hostname =
      origin.hostname === 'localhost' ? '127.0.0.1' : 'localhost'
  }
  return origin.origin
}

async function labState(lab: Lab, page: number): Promise<LabState> {
  const text = expectOk(
    await lab.mcp.callTool('evaluate_in_page', {
      page,
      code: 'return JSON.stringify(window.frameLab)',
    }),
    'read frame lab state',
  )
  const start = text.indexOf('{')
  const end = text.lastIndexOf('}')
  if (start === -1 || end < start) {
    throw new Error(`frame lab state is not JSON: ${text.slice(0, 400)}`)
  }
  return JSON.parse(text.slice(start, end + 1)) as LabState
}

/** Has the document at `level` apply `action` to its own child frame, and waits for it. */
async function labCommand(
  lab: Lab,
  page: number,
  level: number,
  action: 'detach' | 'replace' | 'reload',
): Promise<void> {
  expectOk(
    await lab.mcp.callTool('evaluate_in_page', {
      page,
      code: `return window.frameLab.command(${level}, ${JSON.stringify(action)})`,
    }),
    `frame lab ${action} at level ${level}`,
  )
  await waitUntil(
    async () => (await labState(lab, page)).done.includes(`${level}:${action}`),
    `level ${level} to ${action} its frame`,
  )
}

/** Waits until a new target document has loaded after `previous`. */
async function newTargetDoc(
  lab: Lab,
  page: number,
  previous: string | undefined,
): Promise<string> {
  let doc: string | undefined
  await waitUntil(
    async () => {
      doc = (await labState(lab, page)).targetDoc
      return doc !== undefined && doc !== previous
    },
    'the replacement target document to load',
    { timeoutMs: 30_000 },
  )
  return doc as string
}

async function openLab(lab: Lab, id: string, scroll = false): Promise<number> {
  const page = await lab.ctx.openPage(labUrl(lab, id, scroll), lab.mcp)
  await waitUntil(
    async () => (await labState(lab, page)).ready,
    `frame lab ${id} to load every level`,
    { timeoutMs: 30_000 },
  )
  return page
}

function refFor(snapshot: string, label: string): string | undefined {
  return snapshot
    .split('\n')
    .find((line) => line.includes(label))
    ?.match(/\[ref=(e\d+)\]/)?.[1]
}

/** A full snapshot ref for `label`, retried while late frames attach. */
async function snapshotRef(
  lab: Lab,
  page: number,
  label: string,
): Promise<string> {
  let ref: string | undefined
  let snapshot = ''
  await waitUntil(
    async () => {
      snapshot = expectOk(
        await lab.mcp.callTool('snapshot', { page }),
        'frame lab snapshot',
      )
      ref = refFor(snapshot, label)
      return ref !== undefined
    },
    `a ref for "${label}"`,
    { timeoutMs: 20_000 },
  ).catch((error) => {
    throw new Error(`${error}\n${snapshot.slice(0, 1600)}`)
  })
  return ref as string
}

/** Waits for one new trusted click on `control`, received by the case's target document. */
async function trustedClick(
  lab: Lab,
  page: number,
  labCase: LabCase,
  control: LabEvent['control'],
  seen: number,
): Promise<LabEvent> {
  let state: LabState | undefined
  await waitUntil(async () => {
    state = await labState(lab, page)
    return state.events.length > seen
  }, `a ${control} click in ${labCase.id}`)
  const events = state?.events.slice(seen) ?? []
  const event = events[0]
  if (
    events.length !== 1 ||
    event.control !== control ||
    !event.trusted ||
    event.level !== labCase.level ||
    event.origin !== targetOrigin(lab, labCase)
  ) {
    throw new Error(
      `${labCase.id}: expected one trusted ${control} click at level ${labCase.level} from ${targetOrigin(lab, labCase)}, got ${JSON.stringify(events)}`,
    )
  }
  return event
}

/**
 * A click that should be refused must neither report success nor land anywhere. A frame that
 * is no longer this page's is refused with the typed `frame_not_in_page` code.
 */
async function requireRefusedClick(
  lab: Lab,
  page: number,
  ref: string,
  context: string,
  typed: boolean,
): Promise<string> {
  const before = (await labState(lab, page)).events.length
  const text = expectError(
    await lab.mcp.callTool('act', { page, kind: 'click', ref, diff: 'none' }),
    context,
  )
  const after = (await labState(lab, page)).events
  if (after.length !== before) {
    throw new Error(
      `${context} delivered a click: ${JSON.stringify(after.slice(before))}`,
    )
  }
  if (typed && !text.includes('frame_not_in_page')) {
    throw new Error(`${context} was not refused as frame_not_in_page: ${text}`)
  }
  return text
}

const scenarios = (mode: Mode): ContractCase[] => [
  ...LAB_CASES.map((labCase) => ({
    name: `frame refs (${mode}): ${labCase.id} refs click with trusted input through snapshot, act and run`,
    async run(ctx: CaseContext) {
      const lab = await labIn(ctx, mode)
      const page = await openLab(lab, labCase.id)
      const buttonLabel = `Click target ${labCase.id}`
      const checkboxLabel = `Check target ${labCase.id}`

      const buttonRef = await snapshotRef(lab, page, buttonLabel)
      const interactive = expectOk(
        await lab.mcp.callTool('snapshot', { page, mode: 'interactive' }),
        'interactive frame lab snapshot',
      )
      if (refFor(interactive, buttonLabel) !== buttonRef) {
        throw new Error(
          `interactive snapshot lost ${buttonLabel} [ref=${buttonRef}]:\n${interactive.slice(0, 1600)}`,
        )
      }
      if (!refFor(interactive, checkboxLabel)) {
        throw new Error(`interactive snapshot has no ${checkboxLabel} ref`)
      }

      expectOk(
        await lab.mcp.callTool('act', { page, kind: 'click', ref: buttonRef }),
        `act click ${buttonLabel}`,
      )
      await trustedClick(lab, page, labCase, 'button', 0)

      // The run tool's page.snapshot() and page.click(ref) share the same refs and admission.
      const run = await lab.mcp.callTool('run', {
        code: `const p = browser.page(${page});
          const full = await p.snapshot();
          const compact = await p.snapshot({ mode: 'interactive' });
          const entry = full.refs.find((ref) => ref.name === ${JSON.stringify(checkboxLabel)});
          if (!entry) throw new Error('no checkbox ref in page.snapshot(): ' + full.text);
          if (!compact.text.includes('[ref=' + entry.ref + ']')) throw new Error('interactive page.snapshot() lost ' + entry.ref);
          await p.click(entry.ref);
          return { frameId: entry.frameId ?? null };`,
      })
      expectOk(run, `run page.click ${checkboxLabel}`)
      const event = await trustedClick(lab, page, labCase, 'checkbox', 1)
      if (!event.checked) {
        throw new Error(`${labCase.id}: the checkbox click did not check it`)
      }
      const value = (
        run.structuredContent as { value?: { frameId?: string | null } }
      )?.value
      const framed = typeof value?.frameId === 'string'
      if (framed !== labCase.level > 0) {
        throw new Error(
          `${labCase.id}: page.snapshot() ref frame mismatch: ${JSON.stringify(value)}`,
        )
      }
    },
  })),
  ...['single-cross', 'nested-cross', 'nested-mixed'].map((id) => ({
    name: `frame refs (${mode}): ${id} scrolls nested frames to reach an off-screen ref`,
    async run(ctx: CaseContext) {
      const lab = await labIn(ctx, mode)
      const page = await openLab(lab, id, true)
      if ((await labState(lab, page)).inView) {
        throw new Error(`${id}: the scroll target started in view`)
      }
      const ref = await snapshotRef(lab, page, `Click target ${id}`)
      expectOk(
        await lab.mcp.callTool('act', { page, kind: 'click', ref }),
        `act click off-screen ${id} target`,
      )
      await trustedClick(lab, page, labCase(id), 'button', 0)
      await waitUntil(
        async () => (await labState(lab, page)).inView,
        `${id} target to be scrolled into the top viewport`,
      )
    },
  })),
  {
    name: `frame refs (${mode}): refs of a detached frame are refused, never re-aimed`,
    async run(ctx: CaseContext) {
      const lab = await labIn(ctx, mode)
      // The old ref is used without a new snapshot. An OOPIF that is gone is refused as
      // frame_not_in_page; a same-process frame's old node is simply no longer found.
      for (const [id, level, typed] of [
        ['single-cross', 0, true],
        ['sandboxed', 0, true],
        ['single-same', 0, false],
        ['nested-cross', 1, false],
        ['nested-mixed', 1, true],
      ] as const) {
        const page = await openLab(lab, id)
        const ref = await snapshotRef(lab, page, `Click target ${id}`)
        await labCommand(lab, page, level, 'detach')
        await requireRefusedClick(
          lab,
          page,
          ref,
          `${id} detached frame ref`,
          typed,
        )
      }
    },
  },
  {
    name: `frame refs (${mode}): a replaced frame owner refuses old refs; a new snapshot reaches the new frame`,
    async run(ctx: CaseContext) {
      const lab = await labIn(ctx, mode)
      const target = labCase('nested-mixed')
      const page = await openLab(lab, target.id)
      const label = `Click target ${target.id}`
      const oldRef = await snapshotRef(lab, page, label)
      const oldDoc = (await labState(lab, page)).targetDoc
      // The B frame swaps its iframe element for a new one: a new owner and a new frame.
      await labCommand(lab, page, 1, 'replace')
      const newDoc = await newTargetDoc(lab, page, oldDoc)
      await requireRefusedClick(
        lab,
        page,
        oldRef,
        'replaced owner old ref',
        true,
      )
      const newRef = await snapshotRef(lab, page, label)
      expectOk(
        await lab.mcp.callTool('act', { page, kind: 'click', ref: newRef }),
        'click the replacement frame ref',
      )
      const event = await trustedClick(lab, page, target, 'button', 0)
      if (event.docId !== newDoc) {
        throw new Error(
          `the click landed in ${event.docId}, not the new document ${newDoc}`,
        )
      }
    },
  },
  {
    name: `frame refs (${mode}): a reloaded frame keeps its old refs inside that same frame`,
    async run(ctx: CaseContext) {
      const lab = await labIn(ctx, mode)
      for (const [id, level] of [
        ['single-cross', 0],
        ['nested-mixed', 1],
      ] as const) {
        const target = labCase(id)
        const page = await openLab(lab, id)
        const ref = await snapshotRef(lab, page, `Click target ${id}`)
        const oldDoc = (await labState(lab, page)).targetDoc
        await labCommand(lab, page, level, 'reload')
        const newDoc = await newTargetDoc(lab, page, oldDoc)
        // The frame keeps its owner and endpoint, so the ref is recovered by role and name
        // inside that frame: it must reach the reloaded document, with trusted input.
        expectOk(
          await lab.mcp.callTool('act', { page, kind: 'click', ref }),
          `${id} ref after its frame reloaded`,
        )
        const event = await trustedClick(lab, page, target, 'button', 0)
        if (event.docId !== newDoc) {
          throw new Error(`${id}: reloaded ref clicked ${event.docId}`)
        }
      }
    },
  },
]

export const frameRefCases: ContractCase[] = [
  ...scenarios('unbound'),
  ...scenarios('scoped').map((scenario) => ({ ...scenario, scoped: true })),
]
