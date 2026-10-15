#!/usr/bin/env bun

// Packages the shipped BrowserOS server (apps/server-rust); server.ts still builds the TS one.
import { runCompiledResourceBuild } from '@browseros/build-server-tools'

import { compileServerRustBinaries } from './server-rust/compiler'
import { serverRustBuildProduct } from './server-rust/descriptor'

runCompiledResourceBuild(
  serverRustBuildProduct,
  compileServerRustBinaries,
  process.argv.slice(2),
).catch((error) => {
  const message = error instanceof Error ? error.message : String(error)
  console.error(`\n✗ ${message}\n`)
  process.exit(1)
})
