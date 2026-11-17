/**
 * The inert rule: what Rust marks inert is left out of Neo's format and counted (`# not listed:`), listed under `{ inert: true }`, and headed `# inert:` in `mode: 'full'`.
 */
import type { Gap } from './model'

export const inert: Gap = { snapshots: {}, cases: [] }
