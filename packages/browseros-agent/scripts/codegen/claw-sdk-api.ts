/**
 * Codegen for the claw-sdk API (claw-sdk's host listener, `/v1`). Reads the host-facing
 * spec at `contracts/claw-sdk-api/openapi.yaml` and emits Rust models into
 * `crates/claw-sdk-api/src/generated`. claw-sdk's service routes return only these types,
 * so the wire shape cannot drift from the published spec. The tree is never hand-edited:
 * change the spec, then run `bun run codegen:claw-sdk-api`.
 *
 * `--check` (`bun run codegen:claw-sdk-api:check`, run in CI) regenerates into a temporary
 * directory and fails if the committed tree differs by a byte.
 */

import { join } from 'node:path'
import { runRustModelsCodegen } from './openapi-generator'

if (import.meta.main) {
  runRustModelsCodegen(
    { name: 'claw-sdk-api', label: 'claw-sdk API' },
    join(import.meta.dir, '../..'),
    process.argv,
  )
}
