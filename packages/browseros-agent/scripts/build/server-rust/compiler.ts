/** Compile BrowserOS's Rust server through the same Cargo toolchain as Claw.
 * Native builds also verify the process-level version used by Chromium and OTA;
 * cross builds cannot execute the output on the packaging host.
 */
import { execFileSync } from 'node:child_process'

import type { ProductCompiler } from '@browseros/build-server-tools'

import { compileRustBinaries } from '../claw-server/compiler'

export const compileServerRustBinaries: ProductCompiler = async (
  product,
  targets,
  envVars,
  processEnv,
  version,
) => {
  const compiled = await compileRustBinaries(
    { package: 'server-rust', bin: 'browseros-server-rs' },
    targets,
    { ...processEnv, ...envVars },
  )
  for (const { target, binaryPath } of compiled) {
    if (target.os === 'macos' && target.arch === process.arch) {
      const actual = execFileSync(binaryPath, ['--version'], {
        encoding: 'utf8',
      }).trim()
      if (actual !== version) {
        throw new Error(
          `${product.label}: binary version ${actual} does not match ${version}`,
        )
      }
    }
  }
  return compiled
}
