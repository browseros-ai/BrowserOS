/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Turns recorded tool calls into short, plain sentences for people who are
 * not developers ("Opened Google Flights", "Typed into the sign-in page"),
 * and decides which calls are behind-the-scenes plumbing that should not be
 * listed as steps at all.
 */

import type { SessionBrowserTab } from '@browseros/claw-api'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import type { ReplayVerb } from '@/modules/api/replay.hooks'

export const TOOL_TO_VERB: Record<string, ReplayVerb> = {
  tabs: 'navigate',
  navigate: 'navigate',
  windows: 'navigate',
  tab_groups: 'navigate',
  snapshot: 'read',
  read: 'read',
  grep: 'read',
  diff: 'read',
  screenshot: 'read',
  act: 'click',
  upload: 'attach',
  download: 'attach',
  pdf: 'read',
  wait: 'read',
  run: 'type',
  evaluate: 'type',
  'pages.newPage': 'navigate',
  'nav.goto': 'navigate',
  'nav.back': 'navigate',
  'nav.forward': 'navigate',
  'nav.reload': 'navigate',
  'observe.snapshot': 'read',
  'observe.diff': 'read',
  'input.click': 'click',
  'input.hover': 'click',
  'input.selectOption': 'click',
  'input.scroll': 'click',
  'input.fill': 'type',
  'input.type': 'type',
  'input.insertText': 'type',
  'input.press': 'type',
  pursue: 'browse',
}

export function verbForTool(toolName: string): ReplayVerb {
  // SDK operation names are open-ended; prototype properties are not tool entries.
  return Object.hasOwn(TOOL_TO_VERB, toolName) ? TOOL_TO_VERB[toolName] : 'read'
}

/** Calls that only set up or check the agent's own state. */
const ALWAYS_HIDDEN = new Set([
  'browser_profiles',
  'browser_target',
  'name_session',
  'wait',
  'screenshot',
  'helpers',
  'helpers.save',
  'helpers.list',
  'helpers.read',
  'pages.list',
  'pages.getInfo',
  'pages.close',
  'observe.resolveRef',
  'cdp',
  'cdpJsonForPage',
])

const LISTING_TOOLS = new Set(['tabs', 'windows', 'tab_groups'])
const LISTING_ACTIONS = new Set(['list', 'get', 'active'])

const MAX_PAGE_LABEL = 60

type Args = Record<string, unknown>

function parseJson(json: string | undefined | null): unknown {
  if (!json) return undefined
  try {
    return JSON.parse(json)
  } catch {
    return undefined
  }
}

function isArgs(value: unknown): value is Args {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

function parseArgs(argsJson: string | undefined | null): Args {
  const parsed = parseJson(argsJson)
  return isArgs(parsed) ? parsed : {}
}

/** The `act` kind each script input call performs. */
const INPUT_KINDS: Record<string, string> = {
  'input.click': 'click',
  'input.fill': 'fill',
  'input.type': 'type',
  'input.insertText': 'type',
  'input.press': 'press',
  'input.hover': 'hover',
  'input.selectOption': 'select',
  'input.scroll': 'scroll',
}

interface Call {
  tool: string
  args: Args
}

/**
 * A call as the sentence rules see it. Calls made inside a `run` script are
 * recorded under their wire names with positional args, usually
 * `[pageId, ...]`; each maps onto the tool it mirrors.
 */
function normalizeCall(
  dispatch: Pick<ToolDispatchRow, 'toolName' | 'argsJson'>,
): Call {
  const tool = dispatch.toolName
  const raw = parseJson(dispatch.argsJson)
  if (!Array.isArray(raw)) return { tool, args: isArgs(raw) ? raw : {} }
  const kind = Object.hasOwn(INPUT_KINDS, tool) ? INPUT_KINDS[tool] : undefined
  if (kind === 'press') return { tool: 'act', args: { kind, key: raw[1] } }
  if (kind === 'scroll') {
    return { tool: 'act', args: { kind, direction: raw[1] } }
  }
  if (kind) return { tool: 'act', args: { kind } }
  switch (tool) {
    case 'pages.newPage':
      return { tool: 'navigate', args: { url: raw[0] } }
    case 'nav.goto':
      return { tool: 'navigate', args: { url: raw[1] } }
    case 'observe.snapshot':
      return { tool: 'snapshot', args: {} }
    case 'observe.diff':
      return { tool: 'diff', args: {} }
    default:
      // Tool-shaped calls such as `evaluate` take `[pageId, options]`.
      return { tool, args: isArgs(raw[1]) ? raw[1] : {} }
  }
}

/** The page a call ran on, ignoring query and hash, else its title. */
function pageKeyOf(dispatch: ToolDispatchRow, call: Call): string | undefined {
  const url = dispatch.url ?? stringArg(call.args, 'url')
  if (url) {
    try {
      const parsed = new URL(url)
      return `${parsed.origin}${parsed.pathname}`
    } catch {}
  }
  return dispatch.title?.trim() || undefined
}

/** Whether a call was made from inside a `run` script. */
function isScriptCall(
  dispatch: Pick<ToolDispatchRow, 'argsJson'> | undefined,
): boolean {
  return Array.isArray(parseJson(dispatch?.argsJson))
}

/** Page code that clicks something, the common way scripts press a button. */
function clicksInPage(args: Args): boolean {
  const source = stringArg(args, 'func') ?? stringArg(args, 'code') ?? ''
  return /\.click\(\s*\)/.test(source)
}

function parseMeta(resultMeta: string | undefined | null): Args {
  return parseArgs(resultMeta)
}

/** The audit writer leaves this key present even if the engine returned no run ID. */
export function isSdkRun(dispatch: ToolDispatchRow): boolean {
  return (
    dispatch.toolName === 'run' &&
    dispatch.parentDispatchId === undefined &&
    Object.hasOwn(parseMeta(dispatch.resultMeta), 'sdkRunId')
  )
}

/** A save_skill call: the first save, the flow that makes it ultrafast, or a repair. */
function describeSkillSave(args: Args): string {
  if (args.patch !== undefined) return 'Repaired a step of this skill'
  if (args.flow !== undefined) return 'Made this skill ultrafast'
  return 'Saved this as a skill'
}

/** neo's fast browsing working the page toward the agent's goal. */
function describePursue(args: Args): string {
  const goal = stringArg(args, 'goal')
  return goal
    ? `neo took over the clicking: “${truncate(goal)}”`
    : 'neo took over the clicking'
}

function describeRun(
  dispatch: ToolDispatchRow,
  meta: Args,
  args: Args,
  page: string | undefined,
): string {
  if (!isSdkRun(dispatch)) {
    return page ? `Worked on ${page}` : 'Worked in the browser'
  }
  // The audit writer redacts the script arguments and saves the name with its outcome.
  const taskName = stringArg(meta, 'taskName') ?? stringArg(args, 'taskName')
  return taskName
    ? `Ran a browser script for ${truncate(taskName)}`
    : 'Ran a browser script'
}

function stringArg(args: Args, key: string): string | undefined {
  const value = args[key]
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}

export function hostOf(url: string | undefined | null): string | undefined {
  if (!url) return undefined
  try {
    const host = new URL(url).hostname
    return host.replace(/^www\./, '') || undefined
  } catch {
    return undefined
  }
}

function truncate(label: string): string {
  return label.length > MAX_PAGE_LABEL
    ? `${label.slice(0, MAX_PAGE_LABEL - 1).trimEnd()}…`
    : label
}

/** The page a call ran on, as a person would name it. */
function pageLabel(
  title: string | undefined | null,
  url: string | undefined | null,
): string | undefined {
  const cleanTitle = title?.trim()
  if (cleanTitle) return truncate(cleanTitle)
  return hostOf(url)
}

export function isBehindTheScenes(
  dispatch: Pick<ToolDispatchRow, 'toolName' | 'argsJson'>,
): boolean {
  const { tool, args } = normalizeCall(dispatch)
  if (ALWAYS_HIDDEN.has(tool)) return true
  if (LISTING_TOOLS.has(tool)) {
    const action = stringArg(args, 'action') ?? 'list'
    return LISTING_ACTIONS.has(action)
  }
  return false
}

/** The icon family for one call, matching what its sentence says it did. */
export function verbForCall(
  dispatch: Pick<ToolDispatchRow, 'toolName' | 'argsJson'>,
): ReplayVerb {
  const { tool, args } = normalizeCall(dispatch)
  if (tool === 'act') {
    const kind = stringArg(args, 'kind')
    return kind && ['type', 'type_at', 'fill', 'press'].includes(kind)
      ? 'type'
      : 'click'
  }
  if (tool === 'evaluate') return clicksInPage(args) ? 'click' : 'read'
  return verbForTool(tool)
}

function describeAct(args: Args, onPage: string): string {
  const kind = stringArg(args, 'kind')
  switch (kind) {
    case 'click':
    case 'click_at':
      return `Clicked on ${onPage}`
    // Typed text is never echoed: it can be a password or a code.
    case 'type':
    case 'type_at':
    case 'fill':
      return `Typed into ${onPage}`
    case 'press': {
      const key = stringArg(args, 'key')
      return key ? `Pressed ${key} on ${onPage}` : `Pressed a key on ${onPage}`
    }
    case 'scroll': {
      const direction = stringArg(args, 'direction') ?? 'down'
      return `Scrolled ${direction} on ${onPage}`
    }
    case 'hover':
    case 'hover_at':
    case 'focus':
      return `Moved to an item on ${onPage}`
    case 'check':
      return `Ticked a box on ${onPage}`
    case 'uncheck':
      return `Unticked a box on ${onPage}`
    case 'select':
      return `Picked an option on ${onPage}`
    case 'drag':
    case 'drag_at':
      return `Dragged an item on ${onPage}`
    case 'dialog_accept':
      return `Accepted a pop-up on ${onPage}`
    case 'dialog_dismiss':
      return `Closed a pop-up on ${onPage}`
    default:
      return `Worked on ${onPage}`
  }
}

function describeFileSelection(args: Args, onPage: string): string {
  return Array.isArray(args.files) && args.files.length === 0
    ? `Cleared file selection on ${onPage}`
    : `Selected files on ${onPage}`
}

/** One recorded call as a past-tense sentence. */
export function describeStep(dispatch: ToolDispatchRow): string {
  const meta = parseMeta(dispatch.resultMeta)
  const page = pageLabel(dispatch.title, dispatch.url)
  if (meta.cancellationKind === 'cockpit.operator-cancelled') {
    return 'You stopped the task'
  }
  if (meta.isError === true) {
    const where = hostOf(dispatch.url) ?? page
    return where ? `Something went wrong on ${where}` : 'Something went wrong'
  }

  const { tool, args } = normalizeCall(dispatch)
  const onPage = page ?? 'the page'
  // Chooser interception completes the click, but file selection is a later call.
  // Keep the history from claiming a file was attached before that call succeeds.
  if (meta.fileChooserOpen === true) return `Opened a file chooser on ${onPage}`

  if (
    LISTING_TOOLS.has(tool) &&
    LISTING_ACTIONS.has(stringArg(args, 'action') ?? 'list')
  ) {
    return 'Checked the open tabs'
  }

  switch (tool) {
    case 'tabs': {
      const action = stringArg(args, 'action') ?? 'list'
      if (action === 'close') return page ? `Closed ${page}` : 'Closed a page'
      const target = page ?? hostOf(stringArg(args, 'url'))
      return target ? `Opened ${target}` : 'Opened a new page'
    }
    case 'navigate':
    case 'windows': {
      const target = page ?? hostOf(stringArg(args, 'url'))
      return target ? `Opened ${target}` : 'Opened a new page'
    }
    case 'nav.back':
      return `Went back to ${onPage}`
    case 'nav.forward':
      return `Went forward to ${onPage}`
    case 'nav.reload':
      return `Reloaded ${onPage}`
    case 'snapshot':
    case 'read':
      return `Read ${onPage}`
    case 'evaluate':
      return clicksInPage(args) ? `Clicked on ${onPage}` : `Read ${onPage}`
    case 'history':
      return 'Looked through your browsing history'
    // Behind-the-scenes calls, seen only when the person asks to show them.
    case 'name_session':
      return 'Named the task'
    case 'browser_profiles':
    case 'browser_target':
      return 'Checked which browser profile to use'
    case 'wait':
      return `Waited for ${onPage}`
    case 'screenshot':
      return `Took a picture of ${onPage}`
    case 'pages.list':
    case 'pages.getInfo':
      return 'Checked the open pages'
    case 'pages.close':
      return 'Closed a page it had opened'
    case 'helpers':
    case 'helpers.save':
    case 'helpers.list':
    case 'helpers.read':
      return 'Used its saved shortcuts'
    case 'observe.resolveRef':
      return `Found an item on ${onPage}`
    case 'cdp':
    case 'cdpJsonForPage':
      return 'Sent the browser a direct command'

    case 'grep':
      return `Searched ${onPage}`
    case 'diff':
      return `Checked what changed on ${onPage}`
    case 'pdf':
      return `Saved ${onPage} as a PDF`
    case 'act':
      return describeAct(args, onPage)
    case 'upload':
      return describeFileSelection(args, onPage)
    case 'download':
      return 'Downloaded a file'
    case 'request_human_help':
      return 'Asked you for help'
    case 'await_human_help':
      return 'Waited for you'
    case 'save_skill':
      return describeSkillSave(args)
    case 'pursue':
      return describePursue(args)
    case 'run':
      return describeRun(dispatch, meta, args, page)
    case 'run_skill': {
      const name = stringArg(args, 'name')
      return name ? `Ran ${name} the fast way` : 'Ran a skill the fast way'
    }
    case 'credentials':
      return `Used a saved sign-in on ${onPage}`
    default:
      return page ? `Worked on ${page}` : 'Worked in the browser'
  }
}

export interface DescribedStep {
  /** The plain sentence shown to the person. */
  sentence: string
  verb: ReplayVerb
  /** Dispatches folded into this step, in order. */
  dispatchIds: number[]
  /** When the first folded dispatch completed (epoch ms). */
  firstAt: number
  isError: boolean
  cancelled: boolean
  /** True for behind-the-scenes work, present only when asked to include it. */
  hidden: boolean
  /** The page the step ran on (origin and path), when known. */
  pageKey?: string
  /** neo took the task over here and handed it back partway. */
  handedBack?: boolean
  /** The first step after a hand back, where the agent carries on. */
  afterHandBack?: boolean
  /** neo's run, on the step where neo took the task over. */
  neoRun?: NeoRun
  /** On a step neo took during a run: how long after the takeover it came. */
  neoOffsetMs?: number
}

/** One pursue run: what neo was asked, how it ended, and what it did. */
export interface NeoRun {
  goal?: string
  /** Absent while neo is still working. */
  outcome?: 'completed' | 'handed_back' | 'cancelled'
  durationMs?: number
  /** Calls neo made on the page during the run, behind-the-scenes ones left out. */
  stepCount: number
  pagesRead?: number
  pageTokens?: number
}

export interface DescribedSteps {
  steps: DescribedStep[]
  /** How many recorded calls were left out as behind-the-scenes work. */
  hiddenCount: number
}

/** Correlation retained even when a successful parent is omitted from the rail. */
interface StepContext {
  runKey: string | number
  tabKey: string | undefined
  toolName: string
}

function sdkTabKey(dispatch: ToolDispatchRow): string | undefined {
  if (dispatch.tabId !== undefined) return `tab:${dispatch.tabId}`
  if (dispatch.pageId !== undefined) return `page:${dispatch.pageId}`
  // SDK page openings can lack a tab ID, so a shared sentence cannot identify the tab.
  return undefined
}

function sdkRunKey(
  dispatch: ToolDispatchRow | undefined,
  sdkParents: ReadonlySet<string>,
): StepContext['runKey'] | undefined {
  if (!dispatch) return undefined
  const parent = dispatch.parentDispatchId
  if (parent !== undefined) return sdkParents.has(parent) ? parent : undefined
  if (isSdkRun(dispatch)) return dispatch.dispatchKey ?? dispatch.dispatchId
  return undefined
}

/** Resolve correlation before rendering because records from different runs can interleave. */
function indexRunParents(dispatches: readonly ToolDispatchRow[]) {
  const sdkParents = new Set<string>()
  const parentsWithChildren = new Set<string>()
  for (const dispatch of dispatches) {
    if (dispatch.dispatchKey !== undefined && isSdkRun(dispatch)) {
      sdkParents.add(dispatch.dispatchKey)
    }
    if (dispatch.parentDispatchId !== undefined) {
      parentsWithChildren.add(dispatch.parentDispatchId)
    }
  }
  return { sdkParents, parentsWithChildren }
}

/** Catalog rows retain their sentence-based folding. SDK rows also need a shared run and tab. */
function canFoldStep(
  previous: DescribedStep,
  current: DescribedStep,
  previousContext: StepContext | undefined,
  context: StepContext | undefined,
): boolean {
  if (
    current.afterHandBack ||
    !sameSideOfRun(previous, current.neoOffsetMs !== undefined)
  ) {
    return false
  }
  if (context === undefined) {
    if (previousContext !== undefined) return false
  } else if (
    previousContext?.runKey !== context.runKey ||
    context.tabKey === undefined ||
    previousContext.tabKey !== context.tabKey
  ) {
    return false
  }
  const sameSentence = previous.sentence === current.sentence
  if (
    sameSentence &&
    previous.hidden === current.hidden &&
    (context === undefined ||
      (previousContext?.toolName === context.toolName &&
        previous.isError === current.isError &&
        previous.cancelled === current.cancelled))
  ) {
    return true
  }
  // A read of the page the previous step worked on can be a check of that work.
  return (
    current.verb === 'read' &&
    !current.hidden &&
    !current.isError &&
    !current.cancelled &&
    !previous.hidden &&
    current.pageKey !== undefined &&
    previous.pageKey === current.pageKey &&
    (context === undefined ||
      (!previous.isError && !previous.cancelled && !sameSentence))
  )
}

/**
 * The visible steps of a task: behind-the-scenes calls are dropped,
 * consecutive calls that read the same collapse into one step, and reads
 * can fold into the step that worked on the page. SDK rows also need a known
 * tab and a shared run before they can fold.
 */
export function describeSteps(
  dispatches: readonly ToolDispatchRow[],
  options: { includeHidden?: boolean } = {},
): DescribedSteps {
  const { sdkParents, parentsWithChildren } = indexRunParents(dispatches)
  const steps: DescribedStep[] = []
  let hiddenCount = 0
  let previousContext: StepContext | undefined
  // When the latest hand back ended; the next visible step is where the agent carries on.
  let handBackEndedAt: number | undefined
  const trackRun = runTracker()
  for (const [index, dispatch] of dispatches.entries()) {
    const meta = parseMeta(dispatch.resultMeta)
    const isError = meta.isError === true
    const cancelled = meta.cancellationKind === 'cockpit.operator-cancelled'
    const runKey = sdkRunKey(dispatch, sdkParents)
    const next = dispatches[index + 1]
    // The SDK parent carries the failure even when all its operations succeeded.
    const hidden =
      isBehindTheScenes(dispatch) ||
      (dispatch.toolName === 'run' &&
        (runKey === undefined
          ? isScriptCall(next) && sdkRunKey(next, sdkParents) === undefined
          : !isError &&
            dispatch.dispatchKey !== undefined &&
            parentsWithChildren.has(dispatch.dispatchKey)))
    hiddenCount += Number(hidden)
    if (hidden && !options.includeHidden) continue
    const runMark = trackRun(dispatch, hidden)
    // pursue is stamped with its start, so the calls it made sort after it.
    const afterHandBack =
      handBackEndedAt !== undefined && dispatch.createdAt >= handBackEndedAt
    const context: StepContext | undefined =
      runKey === undefined
        ? undefined
        : { runKey, tabKey: sdkTabKey(dispatch), toolName: dispatch.toolName }
    if (afterHandBack) handBackEndedAt = undefined
    const handsBack = handedBack(dispatch)
    if (handsBack) {
      handBackEndedAt =
        dispatch.createdAt + Math.max(0, dispatch.durationMs ?? 0)
    }
    const current: DescribedStep = {
      sentence: describeStep(dispatch),
      verb: verbForCall(dispatch),
      dispatchIds: [dispatch.dispatchId],
      firstAt: dispatch.createdAt,
      isError,
      cancelled,
      hidden,
      pageKey: pageKeyOf(dispatch, normalizeCall(dispatch)),
      ...(handsBack ? { handedBack: true } : {}),
      ...(afterHandBack ? { afterHandBack: true } : {}),
      ...runMark,
    }
    const previous = steps.at(-1)
    if (previous && canFoldStep(previous, current, previousContext, context)) {
      previous.dispatchIds.push(dispatch.dispatchId)
      if (handsBack) previous.handedBack = true
    } else {
      steps.push(current)
    }
    previousContext = context
  }
  return { steps, hiddenCount }
}

/**
 * Follows pursue runs through the calls in order. A run covers pursue's own window, from its
 * start to its start plus its duration; the calls inside it are the steps neo took.
 */
function runTracker() {
  let run: { record: NeoRun; startsAt: number; endsAt: number } | undefined
  return (
    dispatch: ToolDispatchRow,
    hidden: boolean,
  ): Pick<DescribedStep, 'neoRun' | 'neoOffsetMs'> => {
    if (run && dispatch.createdAt > run.endsAt) run = undefined
    if (dispatch.toolName === 'pursue') {
      const endsAt =
        dispatch.durationMs != null
          ? dispatch.createdAt + Math.max(0, dispatch.durationMs)
          : Number.POSITIVE_INFINITY
      run = { record: neoRunOf(dispatch), startsAt: dispatch.createdAt, endsAt }
      return { neoRun: run.record }
    }
    if (!run) return {}
    if (!hidden) run.record.stepCount += 1
    return { neoOffsetMs: dispatch.createdAt - run.startsAt }
  }
}

/** neo's steps never fold into the takeover, and nothing folds across a run's edge. */
function sameSideOfRun(
  previous: DescribedStep | undefined,
  inRun: boolean,
): boolean {
  return (
    previous !== undefined &&
    !previous.neoRun &&
    (previous.neoOffsetMs !== undefined) === inRun
  )
}

/** What a pursue call recorded: the goal it was given and, once it ended, how it went. */
function neoRunOf(dispatch: ToolDispatchRow): NeoRun {
  const goal = stringArg(parseArgs(dispatch.argsJson), 'goal')
  const meta = parseMeta(dispatch.resultMeta)
  const outcome =
    meta.outcome === 'completed' ||
    meta.outcome === 'handed_back' ||
    meta.outcome === 'cancelled'
      ? meta.outcome
      : undefined
  const count = (value: unknown) =>
    typeof value === 'number' && Number.isFinite(value) ? value : undefined
  return {
    ...(goal ? { goal } : {}),
    ...(outcome ? { outcome } : {}),
    ...(outcome && dispatch.durationMs != null
      ? { durationMs: dispatch.durationMs }
      : {}),
    stepCount: 0,
    ...(count(meta.pagesRead) !== undefined
      ? { pagesRead: count(meta.pagesRead) }
      : {}),
    ...(count(meta.pageTokens) !== undefined
      ? { pageTokens: count(meta.pageTokens) }
      : {}),
  }
}

/** A pursue call that ended with neo handing the task back to the agent. */
function handedBack(dispatch: ToolDispatchRow): boolean {
  return (
    dispatch.toolName === 'pursue' &&
    parseMeta(dispatch.resultMeta).outcome === 'handed_back'
  )
}

/** What an agent is doing right now on a live tab, in the present tense. */
/**
 * The line for a live task whose agent has gone quiet: where it last worked, without claiming it
 * is still busy. The agent may be thinking, waiting on its user, or finished; the session ends
 * when its connection closes.
 */
export function describeIdleStep(
  tab: Pick<SessionBrowserTab, 'title' | 'url'> | undefined,
): string {
  if (!tab) return 'No recent activity'
  return `Last active on ${pageLabel(tab.title, tab.url) ?? 'a page'}`
}

export function describeLiveStep(
  tab: Pick<SessionBrowserTab, 'title' | 'url' | 'lastToolName'> | undefined,
): string {
  if (!tab) return 'Getting started'
  const page = pageLabel(tab.title, tab.url) ?? 'a page'
  const tool = tab.lastToolName
  if (!tool) return `Looking at ${page}`
  if (tool === 'request_human_help' || tool === 'await_human_help') {
    return `Waiting for you on ${page}`
  }
  switch (verbForTool(tool)) {
    case 'navigate':
      return `Opening ${page}`
    case 'click':
    case 'type':
      return `Working on ${page}`
    case 'attach':
      return `Handling a file on ${page}`
    default:
      return `Reading ${page}`
  }
}
