/**
 * Codegen for the hosted API (`/v1`, served only in `BROWSERCLAW_HOSTED_MODE=1`). Reads the
 * partner-facing spec at `contracts/hosted-api/openapi.yaml` and emits Rust models into
 * `crates/hosted-api/src/generated`. claw-server's `/v1` handlers return only these types,
 * so the wire shape cannot drift from the published spec. The tree is never hand-edited:
 * change the spec, then run `bun run codegen:hosted-api`.
 *
 * `--check` (`bun run codegen:hosted-api:check`, run in CI) regenerates into a temporary
 * directory and fails if the committed tree differs by a byte.
 */

import { join } from 'node:path'
import { runRustModelsCodegen } from './openapi-generator'

if (import.meta.main) {
  runRustModelsCodegen(
    { name: 'hosted-api', label: 'hosted API' },
    join(import.meta.dir, '../..'),
    process.argv,
  )
}
