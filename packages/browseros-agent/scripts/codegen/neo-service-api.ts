/**
 * Codegen for the Neo service API (claw-sdk's host listener, `/v1`). Reads the host-facing
 * spec at `contracts/neo-service-api/openapi.yaml` and emits Rust models into
 * `crates/neo-service-api/src/generated`. claw-sdk's service routes return only these types,
 * so the wire shape cannot drift from the published spec. The tree is never hand-edited:
 * change the spec, then run `bun run codegen:neo-service-api`.
 *
 * `--check` (`bun run codegen:neo-service-api:check`, run in CI) regenerates into a temporary
 * directory and fails if the committed tree differs by a byte.
 */

import { cpSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  assertTreeMatches,
  dockerUserMount,
  OPENAPI_GENERATOR_IMAGE,
  runDocker,
  rustfmtFiles,
} from './openapi-generator'

const root = join(import.meta.dir, '../..')
const target = join(root, 'crates/neo-service-api/src/generated')

function generate(outputRoot: string): string {
  runDocker([
    'run',
    '--rm',
    '--user',
    dockerUserMount(),
    '-v',
    `${root}:/local:ro`,
    '-v',
    `${outputRoot}:/out`,
    OPENAPI_GENERATOR_IMAGE,
    'generate',
    '-i',
    '/local/contracts/neo-service-api/openapi.yaml',
    '-g',
    'rust',
    '-o',
    '/out/rust',
    '--global-property',
    'models,supportingFiles,modelDocs=false,modelTests=false,apis=false,apiDocs=false,apiTests=false',
    '--additional-properties',
    'packageName=neo-service-api,packageVersion=1.0.0',
  ])
  const models = join(outputRoot, 'rust/src/models')
  rustfmtFiles(models)
  return models
}

if (import.meta.main) {
  const temporaryRoot = mkdtempSync(join(tmpdir(), 'neo-service-api-codegen-'))
  try {
    const models = generate(temporaryRoot)
    if (process.argv.includes('--check')) {
      assertTreeMatches(models, target, 'Neo service API')
      console.log('Neo service API generated output is current.')
    } else {
      rmSync(target, { recursive: true, force: true })
      cpSync(models, target, { recursive: true })
      console.log('Generated Neo service API Rust models.')
    }
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true })
  }
}
