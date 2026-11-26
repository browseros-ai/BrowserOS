/**
 * The page's half of Rust's ChunkRun (apc-engine-design 5.4 and 5.5): a locator chunk whose tree
 * parts (`role`, `internal:role`, `aria-ref`) Rust evaluates over the APC tree and whose page
 * parts (CSS, text, label, layout, ...) the vendored engine keeps evaluating here.
 *
 *   stage         one page stage. Rust has already matched each tree leaf over the frame's whole
 *                 document and hands the matches over as candidate sets; for this one synchronous
 *                 call they become engines of the world's table, and upstream's own
 *                 `querySelectorAll` runs the stage, so `nth`, `has`, `and`, `or`, `chain` and
 *                 layout keep upstream's roots and order exactly
 *   describePins  the page's words for elements Rust selected: the lines of a strict-mode
 *                 violation Rust decided on its own set
 *
 * A role or ref match is a fact of the element alone, so "the matches inside root R" equals the
 * document-wide matches filtered by containment in R; that is what the call-scoped engines answer.
 * A tree part's error, or an `aria-ref` that names no live node, is also known before any root
 * reaches it; the engines act on such a set only once one does, as upstream's would, so Rust gets
 * upstream's answer from one call.
 */
import { asLocator } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/locatorGenerators'
import {
  type ParsedSelector,
  stringifySelector,
  visitAllSelectorParts,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/selectorParser'
import type { FailureOutput } from './failure-output'
import type {
  DescribePinsRequest,
  PinPreview,
  ReplyTo,
  SetMember,
  StageMatch,
  StageRequest,
  StageSet,
} from './protocol'
import { checkEngines } from './resolve'
import { engineTable, ProbeError, type QueryEngine, type World } from './world'

/**
 * The call-scoped engine names. Rust's ChunkPlan refuses them in agent input, and outside a
 * `stage` call they are not in the table, so no other request can reach them.
 */
const FROM = 'internal:neo-from'
const BELOW = 'internal:neo-below'
const INSIDE = 'internal:neo-inside'

/** The engine's `sdkLanguage` (world.ts `createEngine`), which its own messages print in. */
const SDK_LANGUAGE = 'javascript'

/** A member as this world holds it: an element of its document, or the document itself. */
type Held = Element | Document

/** A candidate set once its members are resolved; an error or stale-ref set as Rust sent it. */
type Candidates = { members: Held[] } | Exclude<StageSet, { members: unknown }>

/**
 * A member while it is live: a pin's element or a handed node that is connected and still in
 * this world's document (design 6.2: a node the page moved into another document is not the one
 * the capture saw), or the document.
 */
function held(
  world: World,
  member: SetMember,
  nodes: readonly unknown[],
): Held | undefined {
  const { document } = world.injected
  if (member === 'document') return document
  const node =
    'pin' in member ? world.pins.live(member.pin) : nodes[member.node]
  return node instanceof Element &&
    node.isConnected &&
    node.ownerDocument === document
    ? node
    : undefined
}

/** Every member live, in order; null when any one is not. */
function allHeld(
  world: World,
  members: readonly SetMember[],
  nodes: readonly unknown[],
): Held[] | null {
  const live: Held[] = []
  for (const member of members) {
    const element = held(world, member, nodes)
    if (!element) return null
    live.push(element)
  }
  return live
}

/** The set a `neo-below` or `neo-inside` body names; any other body is a probe bug. */
function setAt(sets: readonly Candidates[], body: unknown): Candidates {
  const set =
    typeof body === 'string' && /^(0|[1-9]\d*)$/.test(body)
      ? sets[Number(body)]
      : undefined
  if (!set)
    throw new ProbeError(
      'internal',
      `a call-scoped engine body ${JSON.stringify(body)} names none of the stage's ${sets.length} candidate sets`,
    )
  return set
}

/**
 * The members of a set inside `root`: shadow-including containment, as Neo's `aria-ref` engine
 * scopes (`isInsideScope`, where a Document root holds everything in it). `inclusive` keeps the
 * root itself, which is an `aria-ref` part's semantics; a role part's leaves it out, as
 * upstream's role engine queries only below its root. A stale-ref set's body goes to `reached`
 * each time a root reaches it, as `RefMap`'s engine records the refs it cannot resolve.
 */
function scoped(
  world: World,
  sets: readonly Candidates[],
  inclusive: boolean,
  reached: string[],
): QueryEngine {
  const { utils } = world.injected
  return {
    queryAll(root, body) {
      const set = setAt(sets, body)
      // Thrown with the class Rust gave it (index.ts's `classify` reads a ProbeError's own),
      // and only once a root reaches the part, as upstream's role engine throws.
      if ('error' in set)
        throw new ProbeError(set.error.class, set.error.message)
      if ('staleRef' in set) {
        reached.push(set.staleRef)
        return []
      }
      const inside = set.members.filter((member) =>
        member === root
          ? inclusive
          : member instanceof Element && utils.isInsideScope(root, member),
      )
      // Upstream's loop holds the Document as a root too (its first one), typed as an Element.
      return inside as Element[]
    },
  }
}

/**
 * `selector` starting from `from`'s members: one prepended part that answers them whatever root
 * reaches it, so every later part runs per member as upstream runs it per root, while
 * `internal:and`, `internal:or` and layout still evaluate from the call's own root.
 */
function startingFrom(selector: ParsedSelector): ParsedSelector {
  return {
    parts: [{ name: FROM, body: '', source: '' }, ...selector.parts],
    ...(selector.capture === undefined
      ? {}
      : { capture: selector.capture + 1 }),
  }
}

export function stage(
  world: World,
  request: StageRequest,
  root: Node | undefined,
  nodes: readonly unknown[],
): ReplyTo<'stage'> {
  const { injected } = world
  const from = request.from && allHeld(world, request.from, nodes)
  if (from === null) return { kind: 'detached' }
  // A candidate set only ever filters by containment, so a member that is gone matches nothing.
  const sets: Candidates[] = (request.sets ?? []).map((set) =>
    'members' in set
      ? {
          members: set.members.flatMap(
            (member) => held(world, member, nodes) ?? [],
          ),
        }
      : set,
  )
  visitAllSelectorParts(request.selector, (part) => {
    if (part.name === FROM)
      throw new ProbeError(
        'internal',
        `a stage selector names ${FROM}, which only the probe prepends`,
      )
    if (part.name === BELOW || part.name === INSIDE) setAt(sets, part.body)
  })
  const selector = from ? startingFrom(request.selector) : request.selector
  // The query is synchronous, so every engine call of this stage, nested parts included, records
  // into this one list; its first body is the reply's `staleRef`.
  const reached: string[] = []
  const scopedEngines = new Map<string, QueryEngine>([
    [BELOW, scoped(world, sets, false, reached)],
    [INSIDE, scoped(world, sets, true, reached)],
  ])
  // Upstream's loop calls it once, with the call's root, as the selector's first part.
  if (from) scopedEngines.set(FROM, { queryAll: () => from as Element[] })
  const engines = engineTable(injected)
  for (const [name, engine] of scopedEngines) engines.set(name, engine)
  let elements: Element[]
  try {
    checkEngines(injected, selector)
    elements = injected.querySelectorAll(selector, root ?? injected.document)
  } finally {
    // The engines answer this call's sets only: a later request must find them unknown.
    for (const name of scopedEngines.keys()) engines.delete(name)
  }
  if (request.strict && elements.length > 1)
    return {
      kind: 'strict',
      message: injected.strictModeViolationError(request.strict, elements)
        .message,
    }
  const [staleRef] = reached
  return {
    kind: 'staged',
    matches: pinMatches(world, elements),
    ...(staleRef === undefined ? {} : { staleRef }),
  }
}

/**
 * Every element match pinned in one entry, so Rust makes a handle of each before any can be
 * evicted; the document stays a marker. `nth` or `internal:or` keep the query root, which is the
 * document or an element handle's element; nothing else can reach the reply.
 */
function pinMatches(world: World, matches: readonly Node[]): StageMatch[] {
  const { document } = world.injected
  const elements: Element[] = []
  for (const match of matches) {
    if (match === document) continue
    if (!(match instanceof Element))
      throw new ProbeError(
        'internal',
        'a stage matched a node that is neither an element nor this document',
      )
    elements.push(match)
  }
  const pins = world.pins.addAll(elements)
  let next = 0
  return matches.map((match) =>
    match === document ? 'document' : { pin: pins[next++] ?? 0 },
  )
}

/**
 * The words of a strict-mode violation for elements Rust chose, exactly as upstream's
 * `strictModeViolationError` writes them (`_generateSelectors`, injectedScript.ts at v1.64.1):
 * each element's `previewNode` and its `generateSelectorSimple` as `asLocator` prints it, all of
 * them read under the guard FailureOutput puts around that listing, which is rooted at the first
 * element; and the chunk's locator as the message's head prints it. Rust assembles the message
 * (`StrictViolation::message`), with the count it decided, from these.
 */
export function describePins(
  world: World,
  request: DescribePinsRequest,
  _root: Node | undefined,
  nodes: readonly unknown[],
  output: FailureOutput,
): ReplyTo<'describePins'> {
  const { injected } = world
  const elements: Element[] = []
  for (const member of request.members) {
    if (member === 'document')
      throw new ProbeError(
        'internal',
        'describePins describes elements, never the document',
      )
    const element = held(world, member, nodes)
    if (!(element instanceof Element)) return { kind: 'detached' }
    elements.push(element)
  }
  const [first] = elements
  const items: PinPreview[] = first
    ? output.tree(first, () =>
        elements.map((element) => ({
          preview: injected.previewNode(element),
          aka: asLocator(
            SDK_LANGUAGE,
            injected.generateSelectorSimple(element),
          ),
        })),
      )
    : []
  return {
    kind: 'pinsDescribed',
    ...(request.chunk
      ? { locator: asLocator(SDK_LANGUAGE, stringifySelector(request.chunk)) }
      : {}),
    items,
  }
}
