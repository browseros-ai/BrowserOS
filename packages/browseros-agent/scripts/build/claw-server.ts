#!/usr/bin/env bun

import { runCompiledResourceBuild } from '@browseros/build-server-tools'

import { compileClawServerBinaries } from './claw-server/compiler'
import { clawServerBuildProduct } from './claw-server/descriptor'

runCompiledResourceBuild(
  clawServerBuildProduct,
  compileClawServerBinaries,
  process.argv.slice(2),
).catch((error) => {
  const message = error instanceof Error ? error.message : String(error)
  console.error(`\n✗ ${message}\n`)
  process.exit(1)
})
