#!/usr/bin/env bun

import { Command } from 'commander'

import { runCliInstallerUpload, runCliRelease } from './cli/upload'

const program = new Command('cli-upload')
  .description('Upload BrowserOS CLI artifacts to CDN')
  .option(
    '--release',
    'Upload full release (binaries + installers + version.txt)',
  )
  .option('--version <version>', 'Release version (required with --release)')
  .option(
    '--binaries-dir <dir>',
    'Directory containing built archives (required with --release)',
  )
  .parse()

const opts = program.opts<{
  release?: boolean
  version?: string
  binariesDir?: string
}>()

async function main(): Promise<void> {
  if (opts.release) {
    if (!opts.version) {
      throw new Error('--version is required with --release')
    }
    if (!opts.binariesDir) {
      throw new Error('--binaries-dir is required with --release')
    }
    await runCliRelease({
      version: opts.version,
      binariesDir: opts.binariesDir,
    })
  } else {
    await runCliInstallerUpload()
  }
}

main().catch((error) => {
  const message = error instanceof Error ? error.message : String(error)
  console.error(`\n✗ ${message}\n`)
  process.exit(1)
})
