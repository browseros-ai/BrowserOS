/**
 * Codegen for the BrowserOS server API, the contract between the extension and the Rust server
 * in `crates/browseros-server`. Unlike the claw-api contract, the source here is Rust: every
 * handler is registered in one `utoipa-axum` list, so the document cannot list a route the
 * server lacks, or miss one it has. Two steps, neither needing Docker:
 *
 *   Rust handlers and http::dto ──(--print-openapi)──▶ contracts/server-api/openapi.json
 *   contracts/server-api/openapi.json ──(openapi-typescript)──▶ packages/server-api/src/generated/openapi.ts
 *
 * `packages/server-api/src/client.ts` is written by hand over the generated types. Change the
 * Rust types, then run `bun run codegen:server-api`. `--check` (`bun run
 * codegen:server-api:check`) regenerates both files into a temporary directory and fails,
 * naming each stale file, if either differs from the committed one by a byte.
 */

import { spawnSync } from 'node:child_process'
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

const root = join(import.meta.dir, '../..')
const SPEC = 'contracts/server-api/openapi.json'
const TYPES = 'packages/server-api/src/generated/openapi.ts'

/**
 * Runs the server binary's `--print-openapi` action. The toolchain and the cargo build
 * directory come from the caller's environment, so nothing machine-specific lives here.
 */
function printOpenApi(): string {
  const result = spawnSync(
    'cargo',
    [
      'run',
      '-q',
      '-p',
      'browseros-server',
      '--locked',
      '--',
      '--print-openapi',
    ],
    {
      cwd: root,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
      stdio: ['ignore', 'pipe', 'inherit'],
    },
  )
  if (result.error) throw result.error
  if (result.status !== 0) {
    throw new Error(
      `browseros-server --print-openapi exited with ${result.status}`,
    )
  }
  // Re-serialised so the committed bytes depend only on the document, not on how the binary
  // prints it. Today this equals serde_json's pretty output plus the trailing newline.
  return `${JSON.stringify(JSON.parse(result.stdout), null, 2)}\n`
}

/** Produces the client types from a spec file for both codegen and the Bun contract test. */
export function generateServerApiTypes(spec: string): string {
  // openapi-typescript 7 uses the TypeScript 5 compiler API, while the workspace's TypeScript 7
  // package no longer exports it (the same pin as scripts/codegen/claw-api.ts). `bun x --bun`
  // from process.execPath runs the CLI on the Bun that started this script instead of on
  // whatever Node its shebang finds, so the pipeline needs only Bun and cargo.
  const result = spawnSync(
    process.execPath,
    [
      'x',
      '--bun',
      '--package',
      'typescript@5.9.3',
      '--package',
      'openapi-typescript@7.13.0',
      'openapi-typescript',
      spec,
    ],
    {
      cwd: root,
      encoding: 'utf8',
      maxBuffer: 64 * 1024 * 1024,
      stdio: ['ignore', 'pipe', 'inherit'],
    },
  )
  if (result.error) throw result.error
  if (result.status !== 0) {
    throw new Error(`openapi-typescript exited with status ${result.status}`)
  }
  return `// Generated from ${SPEC} by \`bun run codegen:server-api\`. Do not edit.\n${result.stdout}`
}

/** Writes both generated files under `outputRoot`, at their paths relative to the agent root. */
function generate(outputRoot: string): void {
  const spec = join(outputRoot, SPEC)
  const types = join(outputRoot, TYPES)
  mkdirSync(dirname(spec), { recursive: true })
  mkdirSync(dirname(types), { recursive: true })
  writeFileSync(spec, printOpenApi())
  writeFileSync(types, generateServerApiTypes(spec))
}

if (import.meta.main) {
  const temporaryRoot = mkdtempSync(join(tmpdir(), 'server-api-codegen-'))
  try {
    generate(temporaryRoot)
    if (process.argv.includes('--check')) {
      const stale = [SPEC, TYPES].filter((path) => {
        const committed = join(root, path)
        return (
          !existsSync(committed) ||
          !readFileSync(committed).equals(
            readFileSync(join(temporaryRoot, path)),
          )
        )
      })
      if (stale.length > 0) {
        console.error(
          `Generated server API is stale; run \`bun run codegen:server-api\`:\n${stale.map((path) => `  ${path}`).join('\n')}`,
        )
        process.exitCode = 1
      } else {
        console.log('Server API generated output is current.')
      }
    } else {
      for (const path of [SPEC, TYPES]) {
        mkdirSync(dirname(join(root, path)), { recursive: true })
        cpSync(join(temporaryRoot, path), join(root, path))
      }
      console.log(`Generated ${SPEC} and ${TYPES}.`)
    }
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true })
  }
}
