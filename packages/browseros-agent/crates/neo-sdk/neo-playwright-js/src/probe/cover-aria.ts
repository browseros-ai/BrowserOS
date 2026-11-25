/**
 * The per-element ARIA-engine seam for cover detection and private-look drawing checks
 * (apc-engine-design section 8.2).
 *
 * Kept free of runtime imports from `./world` so `private-look.ts` (imported by
 * `private-entry.ts`) stays decoupled from `InjectedScript` construction. Snapshot-time
 * readers (`cover-survey.ts`, `snapshot-inert.ts`) extend this seam in `./snapshot-aria`.
 */
import type { AriaRole } from '../../../neo-playwright/third_party/playwright/packages/isomorphic/ariaSnapshot'
import type { World } from './world'

/** A cover's default-mode snapshot paired with whether that snapshot carries any words. */
export type CoverSnapshot = {
  readonly json: unknown
  readonly hasWords: boolean
}

/** Per-element ARIA reads performed during cover detection and private-look checks. */
export type CoverAria = {
  roleOf(element: Element): AriaRole | null
  nameOf(element: Element): string
  refOf(element: Element): string
  coverSnapshot(element: Element): CoverSnapshot
}

const providers = new WeakMap<World, CoverAria>()

export function coverAriaFor(world: World): CoverAria {
  let provider = providers.get(world)
  if (!provider) {
    provider = createEngineCoverAria(world)
    providers.set(world, provider)
  }
  return provider
}

function createEngineCoverAria(world: World): CoverAria {
  return {
    roleOf: (element) => world.injected.utils.getAriaRole(element),
    nameOf: (element) =>
      world.injected.utils.getElementAccessibleNameText(element, false),
    refOf: (element) => world.refs.forElement(element, world.seq),
    coverSnapshot(element) {
      const json = quietSnapshot(world.injected, element)
      return { json, hasWords: holdsWords(json) }
    },
  }
}

/**
 * The cover's own snapshot, moving nothing the agent's own reads depend on: `default` mode mints
 * no engine ref (the engine's ref counter and `_ariaRef` stay as they were, so the agent's next
 * snapshot numbers its refs as it would have without this answer), and the engine's last
 * snapshot, which its aria-ref engine reads, is put back. Not a marked snapshot: it opens no
 * closed root. The refs an answer prints come from the ref map (`forElement`), never from here.
 */
function quietSnapshot(injected: World['injected'], element: Element): unknown {
  const last: unknown = Reflect.get(injected, '_lastAriaSnapshotForQuery')
  try {
    return injected.ariaSnapshotJSON(element, { mode: 'default' }).json
  } finally {
    Reflect.set(injected, '_lastAriaSnapshotForQuery', last)
  }
}

function holdsWords(node: unknown): boolean {
  if (typeof node === 'string') return node.trim() !== ''
  if (Array.isArray(node)) return node.some(holdsWords)
  if (typeof node !== 'object' || node === null) return false
  for (const key of ['name', 'text', 'children'] as const)
    if (holdsWords(Reflect.get(node, key))) return true
  return false
}
