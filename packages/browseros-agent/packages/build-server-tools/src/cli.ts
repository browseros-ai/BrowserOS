import { Command } from 'commander'

import { resolveTargets } from './targets'
import type {
  AssetBuildArgs,
  AssetBuildProductDescriptor,
  BuildArgs,
  BuildProductDescriptor,
} from './types'

export function parseBuildArgs(
  argv: string[],
  product: BuildProductDescriptor,
): BuildArgs {
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
      product.defaultManifestPath,
    )
    .option('--upload', 'Upload artifact zips to R2')
    .option('--no-upload', 'Skip zip upload to R2')
    .option(
      '--ci',
      'Build local release zip artifacts for CI without R2 and without requiring production env secrets',
    )
  program.parse(argv, { from: 'user' })
  const options = program.opts<{
    target: string
    manifest: string
    upload: boolean
    ci: boolean
  }>()

  const ci = options.ci ?? false
  if (ci && options.upload) {
    throw new Error('--ci cannot be combined with --upload')
  }

  return {
    targets: resolveTargets(options.target),
    manifestPath: options.manifest,
    upload: ci ? false : (options.upload ?? product.defaultUpload ?? true),
    ci,
  }
}

export function parseAssetBuildArgs(
  argv: string[],
  product: AssetBuildProductDescriptor,
): AssetBuildArgs {
  const program = new Command()
  program
    .allowUnknownOption(false)
    .allowExcessArguments(false)
    .exitOverride((error) => {
      throw new Error(error.message)
    })
    .option('--upload', 'Upload the artifact zip to R2')
    .option('--no-upload', 'Skip zip upload to R2')
    .option(
      '--ci',
      'Build the local release zip artifact for CI without R2 and without requiring production env secrets',
    )
  program.parse(argv, { from: 'user' })
  const options = program.opts<{ upload?: boolean; ci?: boolean }>()

  const ci = options.ci ?? false
  if (ci && options.upload) {
    throw new Error('--ci cannot be combined with --upload')
  }

  return {
    upload: ci ? false : (options.upload ?? product.defaultUpload ?? true),
    ci,
  }
}
