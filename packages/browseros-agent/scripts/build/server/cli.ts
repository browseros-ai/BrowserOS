import { Command } from 'commander'

import { resolveTargets } from './targets'
import type { BuildArgs } from './types'

const DEFAULT_MANIFEST_PATH = 'scripts/build/config/server-prod-resources.json'

export function parseBuildArgs(argv: string[]): BuildArgs {
  const program = new Command()
  program
    .allowUnknownOption(false)
    .allowExcessArguments(false)
    .exitOverride((error) => {
      throw new Error(error.message)
    })
    .option('--target <targets>', 'Build target ids or "all"', 'all')
    .option(
      '--manifest <path>',
      'Resource manifest path',
      DEFAULT_MANIFEST_PATH,
    )
    .option('--upload', 'Upload artifact zips to R2')
    .option('--no-upload', 'Skip zip upload to R2')
    .option(
      '--compile-only',
      'Compile binaries only (skip artifact packaging, R2 staging, and upload)',
    )
    .option(
      '--ci',
      'Build local release zip artifacts for CI without R2 and without requiring production env secrets',
    )
  program.parse(argv, { from: 'user' })
  const options = program.opts<{
    target: string
    manifest: string
    upload: boolean
    compileOnly: boolean
    ci: boolean
  }>()

  const compileOnly = options.compileOnly ?? false
  const ci = options.ci ?? false
  if (ci && compileOnly) {
    throw new Error('--ci cannot be combined with --compile-only')
  }
  if (ci && options.upload) {
    throw new Error('--ci cannot be combined with --upload')
  }

  return {
    targets: resolveTargets(options.target),
    manifestPath: options.manifest,
    upload: ci || compileOnly ? false : (options.upload ?? true),
    compileOnly,
    ci,
  }
}
