#!/usr/bin/env bun

import { runCompiledResourceBuild } from '@browseros/build-server-tools'

import { compileRustBinaries } from './rust-compiler'
import {
  browserosServerBinary,
  browserosServerBuildProduct,
} from './server/descriptor'

runCompiledResourceBuild(
  browserosServerBuildProduct,
  (product, targets, envVars, processEnv) =>
    compileRustBinaries({
      binary: browserosServerBinary,
      label: product.label,
      targets,
      env: { ...processEnv, ...envVars },
    }),
  process.argv.slice(2),
).catch((error) => {
  const message = error instanceof Error ? error.message : String(error)
  console.error(`\n✗ ${message}\n`)
  process.exit(1)
})
