/**
 * Target resolution, shared by every request that names a target, and the two requests
 * that only resolve: `resolve` (a lookup or a frame hop) and `pinned` (an element by handle).
 * It never waits. "Not there yet" is the reply `none`, and Rust's retry loop owns the timing.
 */
import type { InjectedScript } from '../../../../crates/browseros-playwright/third_party/playwright/packages/injected/src/injectedScript'
import {
  type ParsedSelector,
  stringifySelector,
  visitAllSelectorParts,
} from '../../../../crates/browseros-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import type {
  Detached,
  NoMatch,
  PinnedRequest,
  ProbeTarget,
  ReplyTo,
  ResolveRequest,
  Strictness,
  StrictViolation,
} from './protocol'
import { editedKind } from './sensitive'
import { engineTable, frameSource, ProbeError, type World } from './world'

/** What resolving a target found: its one element, or why there is none to act on. */
export type Found =
  | { kind: 'one'; element: Element }
  | NoMatch
  | StrictViolation
  | Detached

/**
 * Every element `target` matches now, for reads of all matches. A pin that is gone matches
 * nothing. Refs that can never match again are not reported here: counting them is zero, as
 * upstream's count of a ref is.
 */
export function queryAll(
  world: World,
  target: ProbeTarget,
  root: Node | undefined,
): Element[] {
  const { injected } = world
  if (target === 'document') return [injected.document.documentElement]
  if (target === 'root') {
    const element = attachedRoot(root)
    return element ? [element] : []
  }
  if ('pin' in target) {
    const element = world.pins.live(target.pin)
    return element ? [element] : []
  }
  const { chunk } = target
  checkEngines(injected, chunk)
  return world.refs.query(() =>
    injected.querySelectorAll(chunk, root ?? injected.document),
  ).elements
}

/**
 * One element for `target`: the first match, or the only one under `strict`. `root` is the
 * call's root element (an ElementHandle query); without it a chunk is queried from the
 * document. A pin that was evicted or whose element left the document is `detached`.
 */
export function resolveTarget(
  world: World,
  target: ProbeTarget,
  strictness: Strictness,
  root: Node | undefined,
): Found {
  const { injected } = world
  if (target === 'document')
    return { kind: 'one', element: injected.document.documentElement }
  if (target === 'root') {
    const element = attachedRoot(root)
    return element ? { kind: 'one', element } : { kind: 'detached' }
  }
  if ('pin' in target) {
    const element = world.pins.live(target.pin)
    return element ? { kind: 'one', element } : { kind: 'detached' }
  }
  const { chunk } = target
  checkEngines(injected, chunk)
  const { elements, staleRef } = world.refs.query(() =>
    injected.querySelectorAll(chunk, root ?? injected.document),
  )
  const [first] = elements
  if (first === undefined)
    return staleRef === undefined
      ? { kind: 'none' }
      : { kind: 'none', staleRef }
  if (strictness === 'strict' && elements.length > 1)
    return {
      kind: 'strict',
      message: injected.strictModeViolationError(chunk, elements).message,
    }
  return { kind: 'one', element: first }
}

/**
 * An ElementHandle's node, while it is in its document. A handle is detached for good once
 * its node leaves (Neo's rule; upstream reads a detached node's text): an op on it fails
 * instead of reading what no longer shows.
 */
function attachedRoot(root: Node | undefined): Element | undefined {
  return root instanceof Element && root.isConnected ? root : undefined
}

/**
 * Upstream's engine-name check (InjectedScript.parseSelector), for a chunk the prelude
 * already parsed. `ref=` gets Neo's hint: models write it from other tools, and Neo does
 * not add a third spelling for refs (design D3).
 */
function checkEngines(injected: InjectedScript, chunk: ParsedSelector): void {
  const engines = engineTable(injected)
  visitAllSelectorParts(chunk, (part) => {
    if (engines.has(part.name)) return
    if (part.name === 'ref' && typeof part.body === 'string') {
      const ref = unquote(part.body.trim())
      throw new ProbeError(
        'selector',
        `Unknown selector engine "ref". Use page.getByRef('${ref}') or 'aria-ref=${ref}'.`,
      )
    }
    throw new ProbeError(
      'selector',
      `Unknown engine "${part.name}" while parsing selector ${stringifySelector(chunk)}`,
    )
  })
}

function unquote(text: string): string {
  const quote = text[0]
  return (quote === '"' || quote === "'") &&
    text.endsWith(quote) &&
    text.length > 1
    ? text.slice(1, -1)
    : text
}

export function resolve(
  world: World,
  request: ResolveRequest,
  root: Node | undefined,
): ReplyTo<'resolve'> {
  const found = resolveTarget(world, request.target, request.strictness, root)
  if (found.kind !== 'one') return found
  const { element } = found
  if (request.want === 'element')
    return {
      kind: 'element',
      pin: world.pins.add(element),
      preview: world.injected.previewNode(element),
      sensitive: editedKind(world, element),
    }
  const src = frameSource(element)
  if (src === undefined)
    return { kind: 'notFrame', preview: world.injected.previewNode(element) }
  return {
    kind: 'frame',
    pin: world.pins.add(element),
    key: world.refs.frameKey(element),
    visible: world.injected.utils.isElementVisible(element),
    src,
  }
}

/**
 * The pinned element itself; Rust reads it by handle, to make a handle of it or to enter the
 * iframe it is. Null once the pin is evicted; an element that left the page still answers.
 */
export function pinned(world: World, request: PinnedRequest): Element | null {
  return world.pins.held(request.pin) ?? null
}
