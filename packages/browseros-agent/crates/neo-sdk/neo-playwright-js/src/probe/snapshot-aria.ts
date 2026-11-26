/**
 * The engine snapshot's ref table, as inert marking (`snapshot-inert.ts`) reads it: the one read
 * left of the old `snapshot` request, which Rust no longer sends (apc-engine-design 9.3). The
 * seam stays so a test can hand marking a table of its own; the request and this file leave
 * together.
 */
import { lastSnapshotRefs, type World } from './world'

/** Snapshot ref lookup and element order for the latest tree, hiding engine-internal entries. */
export type SnapshotRefs = {
  elementFor(ref: string): Element | undefined
  elements(): Iterable<Element>
}

/** The snapshot reads inert marking needs. */
export type SnapshotAria = {
  snapshotRefs(): SnapshotRefs
}

const providers = new WeakMap<World, SnapshotAria>()

export function snapshotAriaFor(world: World): SnapshotAria {
  let provider = providers.get(world)
  if (!provider) {
    provider = createEngineSnapshotAria(world)
    providers.set(world, provider)
  }
  return provider
}

function createEngineSnapshotAria(world: World): SnapshotAria {
  return {
    snapshotRefs() {
      const refs = lastSnapshotRefs(world.injected)
      return {
        elementFor: (ref) => refs.get(ref)?.element,
        *elements() {
          for (const { element } of refs.values()) yield element
        },
      }
    },
  }
}
