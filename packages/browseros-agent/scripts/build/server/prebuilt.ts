import { statSync } from 'node:fs'
import { resolve } from 'node:path'
import type { ProductCompiler } from '@browseros/build-server-tools'

import { rustBuildInvocation } from '../rust-compiler'
import { browserosServerBinary } from './descriptor'

/**
 * A compiler that hands the packager binaries Cargo already built elsewhere.
 * The release workflow compiles each target on its own native runner, because
 * the local cross-compiler runs only on macOS, then packages all five here.
 * Each binary sits at `<dir>/<target id>/browseros_server[.exe]`; a relative
 * `dir` resolves against `packages/browseros-agent`, the packager's working
 * directory.
 */
export function prebuiltServerCompiler(dir: string): ProductCompiler {
  return async (product, targets) => {
    const compiled = targets.map((target) => ({
      target,
      binaryPath: resolve(
        dir,
        target.id,
        rustBuildInvocation(target, browserosServerBinary).binaryName,
      ),
    }))
    const missing = compiled.filter(
      ({ binaryPath }) =>
        !statSync(binaryPath, { throwIfNoEntry: false })?.isFile(),
    )
    if (missing.length > 0) {
      throw new Error(
        `${product.label}: missing prebuilt binaries:\n${missing
          .map(({ target, binaryPath }) => `- ${target.id}: ${binaryPath}`)
          .join('\n')}`,
      )
    }
    return compiled
  }
}
