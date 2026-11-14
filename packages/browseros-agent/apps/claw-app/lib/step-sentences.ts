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

/** A redacted run can stand alone, with a task name only when its audit row carries one. */
function describeRun(args: Args): string {
  const taskName = stringArg(args, 'taskName')
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
      return describeRun(args)
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
}

export interface DescribedSteps {
  steps: DescribedStep[]
  /** How many recorded calls were left out as behind-the-scenes work. */
  hiddenCount: number
}

/** Correlation retained even when a successful parent is omitted from the rail. */
interface StepContext {
  runKey: ToolDispatchRow['dispatchKey'] | number
  tabKey: string | undefined
  toolName: string
}

function tabKeyOf(
  dispatch: ToolDispatchRow,
  runKey: StepContext['runKey'],
  pageKey: string | undefined,
): string | undefined {
  if (dispatch.tabId !== undefined) return `tab:${dispatch.tabId}`
  if (dispatch.pageId !== undefined) return `page:${dispatch.pageId}`
  // SDK arguments and page labels are redacted. Missing ids must not collapse
  // page openings. Unparented legacy tools retain their page-label fallback.
  if (dispatch.parentDispatchId === undefined && runKey === undefined) {
    return pageKey
  }
  return undefined
}

function sameTabAndRun(
  previous: StepContext | undefined,
  current: StepContext,
): boolean {
  return (
    previous?.runKey === current.runKey &&
    current.tabKey !== undefined &&
    previous?.tabKey === current.tabKey
  )
}

/**
 * The visible steps of a task: behind-the-scenes calls are dropped,
 * consecutive calls for the same operation, run and known tab collapse into
 * one step, and a read of the page the previous step worked on folds into
 * that step within the same run and tab.
 */
export function describeSteps(
  dispatches: readonly ToolDispatchRow[],
  options: { includeHidden?: boolean } = {},
): DescribedSteps {
  const steps: DescribedStep[] = []
  let hiddenCount = 0
  let runKey: StepContext['runKey']
  let previousContext: StepContext | undefined
  // When the latest hand back ended; the next visible step is where the agent carries on.
  let handBackEndedAt: number | undefined
  for (const [index, dispatch] of dispatches.entries()) {
    const meta = parseMeta(dispatch.resultMeta)
    const isError = meta.isError === true
    const cancelled = meta.cancellationKind === 'cockpit.operator-cancelled'
    if (dispatch.toolName === 'run') {
      runKey = dispatch.dispatchKey ?? dispatch.dispatchId
    }
    // Successful scripts are told through their calls. A failed parent carries
    // the script's outcome even when every child operation succeeded.
    const hidden =
      isBehindTheScenes(dispatch) ||
      (dispatch.toolName === 'run' &&
        !isError &&
        isScriptCall(dispatches[index + 1]))
    hiddenCount += Number(hidden)
    if (hidden && !options.includeHidden) continue
    const sentence = describeStep(dispatch)
    const verb = verbForCall(dispatch)
    // pursue is stamped with its start, so the calls it made sort after it.
    const afterHandBack =
      handBackEndedAt !== undefined && dispatch.createdAt >= handBackEndedAt
    const pageKey = pageKeyOf(dispatch, normalizeCall(dispatch))
    const context: StepContext = {
      runKey: dispatch.parentDispatchId ?? runKey,
      tabKey: tabKeyOf(dispatch, runKey, pageKey),
      toolName: dispatch.toolName,
    }
    const sameContext = sameTabAndRun(previousContext, context)
    const previous = steps.at(-1)
    const repeats =
      sameContext &&
      previousContext?.toolName === dispatch.toolName &&
      previous?.sentence === sentence &&
      previous.hidden === hidden &&
      previous.isError === isError &&
      previous.cancelled === cancelled
    // Reading the page it just worked on is the agent checking its own work,
    // not a step of its own.
    const checksPrevious =
      sameContext &&
      verb === 'read' &&
      !hidden &&
      !isError &&
      !cancelled &&
      previous !== undefined &&
      !previous.hidden &&
      !previous.isError &&
      !previous.cancelled &&
      previous.sentence !== sentence &&
      pageKey !== undefined &&
      previous.pageKey === pageKey
    if (afterHandBack) handBackEndedAt = undefined
    const handsBack = handedBack(dispatch)
    if (handsBack) {
      handBackEndedAt =
        dispatch.createdAt + Math.max(0, dispatch.durationMs ?? 0)
    }
    if (previous && !afterHandBack && (repeats || checksPrevious)) {
      previous.dispatchIds.push(dispatch.dispatchId)
      if (handsBack) previous.handedBack = true
      previousContext = context
      continue
    }
    steps.push({
      sentence,
      verb,
      dispatchIds: [dispatch.dispatchId],
      firstAt: dispatch.createdAt,
      isError,
      cancelled,
      hidden,
      pageKey,
      ...(handsBack ? { handedBack: true } : {}),
      ...(afterHandBack ? { afterHandBack: true } : {}),
    })
    previousContext = context
  }
  return { steps, hiddenCount }
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
