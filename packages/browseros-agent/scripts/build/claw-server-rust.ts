#!/usr/bin/env bun

import { runCompiledResourceBuild } from '@browseros/build-server-tools'

import { compileClawServerBinaries } from './claw-server-rust/compiler'
import { clawServerRustBuildProduct } from './claw-server-rust/descriptor'

runCompiledResourceBuild(
  clawServerRustBuildProduct,
  compileClawServerBinaries,
  process.argv.slice(2),
).catch((error) => {
  const message = error instanceof Error ? error.message : String(error)
  console.error(`\n✗ ${message}\n`)
  process.exit(1)
})
