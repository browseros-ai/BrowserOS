/**
 * The bundle entry `dist/attach.js`: `(key) => glue object | undefined`. A few hundred bytes,
 * sent by a run on its first page call in a document, so a run reuses the InjectedScript an
 * earlier run (any session) built there instead of resending the much larger install. Each run
 * still gets its own `objectId` for the glue object, because object ids are per CDP session.
 */

import type { GlueObject, NeoWorld } from './install'
import { WORLD_KEY } from './world-key'

export default function attach(key: string): GlueObject | undefined {
  return (globalThis as NeoWorld)[WORLD_KEY]?.[key]
}
