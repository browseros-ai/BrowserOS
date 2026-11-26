/**
 * Target resolution, shared by every request that names a target, and the two requests
 * that only resolve: `resolve` (a lookup or a frame hop) and `pinned` (an element by handle).
 * It never waits. "Not there yet" is the reply `none`, and Rust's retry loop owns the timing.
 */
import type { InjectedScript } from '../../../neo-playwright/third_party/playwright/packages/injected/src/injectedScript'
import { asLocator } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/locatorGenerators'
import {
  type ParsedSelector,
  stringifySelector,
  visitAllSelectorParts,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import type {
  Detached,
  NoMatch,
  PinnedRequest,
  ProbeTarget,
  ReplyTo,
  Resolution,
  ResolvedSet,
  ResolveRequest,
  SelectorRequest,
  SetMember,
  Strictness,
  StrictViolation,
} from './protocol'
import { engineTable, frameSource, ProbeError, type World } from './world'

/** What resolving a target found: its one element, or why there is none to act on. */
/** `count` is how many a chunk matched; a pin, a root or the document leave it out. */
export type Found =
  | { kind: 'one'; element: Element; count?: number }
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
  // A chunk Rust resolved over the APC tree: its matches still in this document, as a fresh
  // query would find them. A stale ref counts nothing, as the ref engine's does.
  if ('set' in target)
    return target.set.members.flatMap(
      (member) => setMember(world, member) ?? [],
    )
  const { chunk } = target
  checkEngines(injected, chunk)
  return world.refs.query(() =>
    injected.querySelectorAll(chunk, root ?? injected.document),
  ).elements
}

/**
 * A member of a set Rust resolved (`ProbeTarget::Set`): a pinned element while it is connected
 * and still of this world's document, or the document's own root for the Document a chunk's
 * `nth` or `internal:or` kept. A handed node (`node`, or an `option` of a handed select) never
 * reaches a target: Rust pins a tree set before an op names it.
 */
function setMember(world: World, member: SetMember): Element | undefined {
  const { document } = world.injected
  if (member === 'document') return document.documentElement
  if ('node' in member || 'option' in member)
    throw new ProbeError(
      'internal',
      'a resolved set names a handed node, which only a stage takes',
    )
  const element = world.pins.live(member.pin)
  return element?.ownerDocument === document ? element : undefined
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
  if ('set' in target) return resolvedSet(world, target.set, strictness)
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
    return strictViolation(world, chunk, elements)
  return { kind: 'one', element: first, count: elements.length }
}

/** The engine's `sdkLanguage` (world.ts `createEngine`), which its messages print in. */
const SDK_LANGUAGE = 'javascript'

/**
 * Upstream's strict-mode violation for `elements` of `chunk`, with what its message lists: the
 * first ten elements pinned, each with the preview and `aka` the message prints for it. The page's
 * selector generator has no role engine, so Rust replaces an `aka` with the tree's role locator
 * where upstream's generator would have chosen one (apc-engine-design 5.5).
 */
export function strictViolation(
  world: World,
  chunk: ParsedSelector,
  elements: Element[],
): StrictViolation {
  const { injected } = world
  const listed = elements.slice(0, 10)
  return {
    kind: 'strict',
    message: injected.strictModeViolationError(chunk, elements).message,
    listing: {
      locator: asLocator(SDK_LANGUAGE, stringifySelector(chunk)),
      count: elements.length,
      pins: world.pins.addAll(listed),
      items: listed.map((element) => ({
        preview: injected.previewNode(element),
        aka: asLocator(SDK_LANGUAGE, injected.generateSelectorSimple(element)),
      })),
    },
  }
}

/**
 * A chunk Rust resolved over the APC tree, answered as the page's own query of the chunk would
 * be: its first match with how many there were, `none` (with the stale ref the chunk reported)
 * when it matched nothing, and `detached` once a match has left its document, which makes Rust
 * capture again. Rust decided strict mode before the request, so a strict set holds one match.
 */
function resolvedSet(
  world: World,
  set: ResolvedSet,
  strictness: Strictness,
): Found {
  const elements: Element[] = []
  for (const member of set.members) {
    const element = setMember(world, member)
    if (!element) return { kind: 'detached' }
    elements.push(element)
  }
  const [first] = elements
  if (first === undefined)
    return set.staleRef === undefined
      ? { kind: 'none' }
      : { kind: 'none', staleRef: set.staleRef }
  if (strictness === 'strict' && elements.length > 1)
    throw new ProbeError(
      'internal',
      `a strict set holds ${elements.length} elements; Rust decides strict mode`,
    )
  return { kind: 'one', element: first, count: elements.length }
}

/** The reply field naming what a chunk resolved to; nothing for any other target. */
export function resolutionOf(
  world: World,
  found: { element: Element; count?: number },
): { resolved?: Resolution } {
  return found.count === undefined
    ? {}
    : {
        resolved: {
          preview: world.injected.previewNode(found.element),
          count: found.count,
        },
      }
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
export function checkEngines(
  injected: InjectedScript,
  chunk: ParsedSelector,
): void {
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
      ...resolutionOf(world, found),
    }
  const src = frameSource(element)
  if (src === undefined)
    return { kind: 'notFrame', preview: world.injected.previewNode(element) }
  return {
    kind: 'frame',
    pin: world.pins.addFrame(element),
    key: world.refs.frameKey(element),
    visible: world.injected.utils.isElementVisible(element),
    src,
  }
}

/**
 * `locator.normalize`'s part in one frame: upstream's `generateSelectorSimple` for the first
 * element `target` matches now (server/frames.ts `resolveSelector`, which queries without
 * waiting and without strict mode).
 */
export function generatedSelector(
  world: World,
  request: SelectorRequest,
  root: Node | undefined,
): ReplyTo<'selector'> {
  const found = resolveTarget(world, request.target, 'firstMatch', root)
  if (found.kind !== 'one')
    return found.kind === 'strict' ? { kind: 'none' } : found
  return {
    kind: 'selector',
    selector: world.injected.generateSelectorSimple(found.element),
  }
}

/**
 * The pinned element itself; Rust reads it by handle, to make a handle of it or to enter the
 * iframe it is. Null once the pin is evicted; an element that left the page still answers.
 */
export function pinned(world: World, request: PinnedRequest): Element | null {
  return world.pins.held(request.pin) ?? null
}
