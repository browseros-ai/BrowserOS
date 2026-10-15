#!/usr/bin/env bun

// Opt-in packaging entrypoint. The server release continues to invoke server.ts.
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
