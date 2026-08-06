import { Command } from 'commander'

import { resolveTargets } from './targets'
import type {
  AssetBuildArgs,
  AssetBuildProductDescriptor,
  BuildArgs,
  ResourceBuildProductDescriptor,
} from './types'

export function parseBuildArgs(
  argv: string[],
  product: ResourceBuildProductDescriptor,
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
    .option('--versioned-only', 'Upload only immutable versioned R2 keys')
    .option(
      '--ci',
      'Build local release zip artifacts for CI without R2 and without requiring production env secrets',
    )
  program.parse(argv, { from: 'user' })
  const options = program.opts<{
    target: string
    manifest: string
    upload: boolean
    versionedOnly: boolean
    ci: boolean
  }>()

  const ci = options.ci ?? false
  if (ci && options.upload) {
    throw new Error('--ci cannot be combined with --upload')
  }
  const upload = ci ? false : (options.upload ?? product.defaultUpload ?? true)
  const versionedOnly = options.versionedOnly ?? false
  if (versionedOnly && !upload) {
    throw new Error('--versioned-only requires upload')
  }

  return {
    targets: resolveTargets(options.target),
    manifestPath: options.manifest,
    upload,
    versionedOnly,
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
    .option('--versioned-only', 'Upload only the immutable versioned R2 key')
    .option(
      '--ci',
      'Build the local release zip artifact for CI without R2 and without requiring production env secrets',
    )
  program.parse(argv, { from: 'user' })
  const options = program.opts<{
    upload?: boolean
    versionedOnly?: boolean
    ci?: boolean
  }>()

  const ci = options.ci ?? false
  if (ci && options.upload) {
    throw new Error('--ci cannot be combined with --upload')
  }
  const upload = ci ? false : (options.upload ?? product.defaultUpload ?? true)
  const versionedOnly = options.versionedOnly ?? false
  if (versionedOnly && !upload) {
    throw new Error('--versioned-only requires upload')
  }

  return {
    upload,
    versionedOnly,
    ci,
  }
}
