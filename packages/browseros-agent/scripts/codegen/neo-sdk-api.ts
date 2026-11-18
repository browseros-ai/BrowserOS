/**
 * Codegen for the claw-sdk API (claw-sdk's host listener, `/v1`). Reads the host-facing
 * spec at `crates/neo-sdk/neo-sdk-api/openapi.yaml` and emits Rust models into
 * `crates/neo-sdk/neo-sdk-api/src/generated`. claw-sdk's service routes return only these types,
 * so the wire shape cannot drift from the published spec. The tree is never hand-edited:
 * change the spec, then run `bun run codegen:neo-sdk-api`.
 *
 * `--check` (`bun run codegen:neo-sdk-api:check`, run in CI) regenerates into a temporary
 * directory and fails if the committed tree differs by a byte.
 */

import { join } from 'node:path'
import { runRustModelsCodegen } from './openapi-generator'

if (import.meta.main) {
  runRustModelsCodegen(
    {
      name: 'neo-sdk-api',
      label: 'claw-sdk API',
      spec: 'crates/neo-sdk/neo-sdk-api/openapi.yaml',
      crateDir: 'crates/neo-sdk/neo-sdk-api',
    },
    join(import.meta.dir, '../..'),
    process.argv,
  )
}
