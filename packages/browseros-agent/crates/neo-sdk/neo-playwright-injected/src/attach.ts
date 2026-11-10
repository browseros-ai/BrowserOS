/**
 * The bundle entry `dist/attach.js`: `(key) => glue object | undefined`. About a kilobyte, sent
 * by a run on its first page call in a document, so a run reuses the InjectedScript an earlier
 * run (any session) built there instead of resending the much larger install. Each run still gets
 * its own `objectId` for the glue object, because object ids are per CDP session.
 *
 * It also hides BrowserOS's own UI from the world (`browseros-ui.ts`) before anything reads the
 * page. Here, not in `install.ts`: every run's first call in a document passes through attach,
 * and a glue object an earlier Neo build installed under the same key never ran this build's
 * install.
 */

import { hideBrowserOSUi } from './browseros-ui'
import type { GlueObject, NeoWorld } from './install'
import { WORLD_KEY } from './world-key'

export default function attach(key: string): GlueObject | undefined {
  hideBrowserOSUi(globalThis)
  return (globalThis as NeoWorld)[WORLD_KEY]?.[key]
}
