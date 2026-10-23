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
}

export function verbForTool(toolName: string): ReplayVerb {
  return TOOL_TO_VERB[toolName] ?? 'read'
}

/** Calls that only set up or check the agent's own state. */
const ALWAYS_HIDDEN = new Set([
  'browser_profiles',
  'browser_target',
  'name_session',
  'wait',
  'screenshot',
  'helpers',
])

const LISTING_TOOLS = new Set(['tabs', 'windows', 'tab_groups'])
const LISTING_ACTIONS = new Set(['list', 'get', 'active'])

const MAX_PAGE_LABEL = 60

type Args = Record<string, unknown>

function parseArgs(argsJson: string | undefined | null): Args {
  if (!argsJson) return {}
  try {
    const parsed: unknown = JSON.parse(argsJson)
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? (parsed as Args)
      : {}
  } catch {
    return {}
  }
}

function parseMeta(resultMeta: string | undefined | null): Args {
  return parseArgs(resultMeta)
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
  if (ALWAYS_HIDDEN.has(dispatch.toolName)) return true
  if (LISTING_TOOLS.has(dispatch.toolName)) {
    const action = stringArg(parseArgs(dispatch.argsJson), 'action') ?? 'list'
    return LISTING_ACTIONS.has(action)
  }
  return false
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

  const args = parseArgs(dispatch.argsJson)
  const onPage = page ?? 'the page'
  const tool = dispatch.toolName

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
    case 'snapshot':
    case 'read':
      return `Read ${onPage}`
    case 'grep':
      return `Searched ${onPage}`
    case 'diff':
      return `Checked what changed on ${onPage}`
    case 'pdf':
      return `Saved ${onPage} as a PDF`
    case 'act':
      return describeAct(args, onPage)
    case 'upload':
      return `Uploaded a file to ${onPage}`
    case 'download':
      return 'Downloaded a file'
    case 'request_human_help':
      return 'Asked you for help'
    case 'await_human_help':
      return 'Waited for you'
    case 'save_skill':
      return 'Saved this as a saved task'
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
}

export interface DescribedSteps {
  steps: DescribedStep[]
  /** How many recorded calls were left out as behind-the-scenes work. */
  hiddenCount: number
}

/**
 * The visible steps of a task: behind-the-scenes calls are dropped and
 * consecutive calls that read the same ("Read Google Flights" five times)
 * collapse into one step.
 */
export function describeSteps(
  dispatches: readonly ToolDispatchRow[],
): DescribedSteps {
  const steps: DescribedStep[] = []
  let hiddenCount = 0
  for (const dispatch of dispatches) {
    if (isBehindTheScenes(dispatch)) {
      hiddenCount += 1
      continue
    }
    const sentence = describeStep(dispatch)
    const meta = parseMeta(dispatch.resultMeta)
    const isError = meta.isError === true
    const cancelled = meta.cancellationKind === 'cockpit.operator-cancelled'
    const previous = steps.at(-1)
    if (previous && previous.sentence === sentence) {
      previous.dispatchIds.push(dispatch.dispatchId)
      continue
    }
    steps.push({
      sentence,
      verb: verbForTool(dispatch.toolName),
      dispatchIds: [dispatch.dispatchId],
      firstAt: dispatch.createdAt,
      isError,
      cancelled,
    })
  }
  return { steps, hiddenCount }
}

/** What an agent is doing right now on a live tab, in the present tense. */
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
