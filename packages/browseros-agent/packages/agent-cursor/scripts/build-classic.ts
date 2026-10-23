import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const packageRoot = resolve(import.meta.dir, '..')
const cuaCommit = '4a44f4d39677c2d906d588b5b205a85aa8353354'

export async function buildClassic(): Promise<string> {
  const result = await Bun.build({
    entrypoints: [resolve(packageRoot, 'scripts/classic-entry.ts')],
    root: packageRoot,
    target: 'browser',
    format: 'cjs',
    minify: false,
    sourcemap: 'none',
  })
  if (!result.success) {
    throw new AggregateError(result.logs, 'Could not build the cursor asset')
  }
  if (result.outputs.length !== 1) {
    throw new Error('Expected exactly one cursor asset')
  }
  const bundle = await result.outputs[0].text()
  const license = readFileSync(resolve(packageRoot, 'LICENSE'), 'utf8')

  // Keep CommonJS bookkeeping local to the function that loads the cursor.
  return `/*
${license}
Provenance: Cua commit ${cuaCommit}.
*/
var BrowserOSAgentCursor = (() => {
"use strict";
const module = { exports: {} };
const exports = module.exports;
${bundle.trimEnd()}
return module.exports;
})();
`
}

if (import.meta.main) {
  await Bun.write(
    resolve(
      packageRoot,
      '../../crates/browseros-core/src/assets/agent-cursor.js',
    ),
    await buildClassic(),
  )
}
