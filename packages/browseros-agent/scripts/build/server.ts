#!/usr/bin/env bun

import {
  type ProductCompiler,
  runCompiledResourceBuild,
} from '@browseros/build-server-tools'

import { compileRustBinaries } from './rust-compiler'
import {
  browserosServerBinary,
  browserosServerBuildProduct,
  browserosServerCargoEnv,
} from './server/descriptor'
import { prebuiltServerCompiler } from './server/prebuilt'

// BROWSEROS_SERVER_PREBUILT_DIR switches off Cargo. The release workflow sets it
// after compiling each target on a native runner (the local cross-compiler needs
// macOS), so this one packaging run only stages, zips and uploads. Staging, the
// R2 Bun download, artifact metadata and the immutable upload are the same in
// both modes, which keeps release zips identical to what finalize verifies.
const prebuiltDir = process.env.BROWSEROS_SERVER_PREBUILT_DIR?.trim()

const compiler: ProductCompiler = prebuiltDir
  ? prebuiltServerCompiler(prebuiltDir)
  : (product, targets, envVars, processEnv) =>
      compileRustBinaries({
        binary: browserosServerBinary,
        label: product.label,
        targets,
        env: { ...processEnv, ...envVars, ...browserosServerCargoEnv },
      })

runCompiledResourceBuild(
  browserosServerBuildProduct,
  compiler,
  process.argv.slice(2),
).catch((error) => {
  const message = error instanceof Error ? error.message : String(error)
  console.error(`\n✗ ${message}\n`)
  process.exit(1)
})
