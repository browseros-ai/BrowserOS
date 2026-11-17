/**
 * The on-top lines: a layer Rust words is printed first, a whole-view layer's content moves to the front, and a delta says `# no longer on top:`.
 */
import type { Gap } from './model'

export const onTop: Gap = { snapshots: {}, cases: [] }
