/**
 * The snapshot-time ARIA-engine seam for the snapshot-on-top survey (`cover-survey.ts`) and
 * inert marking (`snapshot-inert.ts`), extending `CoverAria` with reads over the latest
 * snapshot's ref table and node links (apc-engine-design section 8.2).
 */
import type { AriaNodeJSON, AriaSnapshotJSON } from '../prelude/vendored'
import { type CoverAria, coverAriaFor } from './cover-aria'
import { lastSnapshotNodes, lastSnapshotRefs, type World } from './world'

/** Snapshot ref lookup and element order for the latest tree, hiding engine-internal entries. */
export type SnapshotRefs = {
  elementFor(ref: string): Element | undefined
  elements(): Iterable<Element>
}

/** Per-element ARIA reads plus snapshot-tree reads needed during a snapshot request. */
export type SnapshotAria = CoverAria & {
  surveyElements(): Iterable<Element>
  snapshotRefs(): SnapshotRefs
  snapshotNodes(json: AriaSnapshotJSON): ReadonlyMap<AriaNodeJSON, Element>
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
  const base = coverAriaFor(world)
  return {
    ...base,
    surveyElements() {
      const refs = lastSnapshotRefs(world.injected)
      return {
        *[Symbol.iterator]() {
          for (const { element } of refs.values()) yield element
        },
      }
    },
    snapshotRefs() {
      const refs = lastSnapshotRefs(world.injected)
      return {
        elementFor: (ref) => refs.get(ref)?.element,
        *elements() {
          for (const { element } of refs.values()) yield element
        },
      }
    },
    snapshotNodes: (json) => lastSnapshotNodes(world.injected, json).nodes,
  }
}
