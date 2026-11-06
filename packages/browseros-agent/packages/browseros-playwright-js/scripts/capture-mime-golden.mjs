/**
 * Records tests/data/mime-golden.json from Playwright's own bundle, so the vendored `mime`
 * package (crates/browseros-playwright/third_party/mime) is tested against the table Playwright
 * types payloads with, not against a reading of its source:
 *
 *   mime     the `mime@<version>` playwright-core's utilsBundle.js.LICENSE names
 *   types    the bundle's `mime` instance, extension to type (`_getTestState().types`), which
 *            is everything `getType` looks a name up in
 *   getType  what that instance's `getType` answers for names that exercise its parsing: case,
 *            path separators, several dots, no dot, a leading dot, a trailing dot
 *
 *   PLAYWRIGHT_NODE_MODULES=<node_modules with playwright-core 1.64.1> \
 *   node scripts/capture-mime-golden.mjs [--check]
 *
 * --check fails if the file would change. playwright-core's version must equal
 * crates/browseros-playwright/third_party/playwright/VERSION.
 */
import { readFileSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const golden = resolve(here, '../tests/data/mime-golden.json')
const vendored = resolve(
  here,
  '../../../crates/browseros-playwright/third_party/playwright/VERSION',
)

const nodeModules = process.env.PLAYWRIGHT_NODE_MODULES
if (!nodeModules) {
  console.error('capture-mime-golden: set PLAYWRIGHT_NODE_MODULES')
  process.exit(2)
}
const require = createRequire(resolve(nodeModules, 'capture.cjs'))
const { version } = require('playwright-core/package.json')
const tag = readFileSync(vendored, 'utf8').split('\n')[0]
if (`v${version}` !== tag) {
  console.error(
    `capture-mime-golden: playwright-core is ${version}; the vendored engine is ${tag}`,
  )
  process.exit(2)
}
const bundle = require.resolve('playwright-core/lib/utilsBundle')
const license = readFileSync(`${bundle}.LICENSE`, 'utf8')
const named = /^- mime@(\d+\.\d+\.\d+) /m.exec(license)
if (!named) {
  console.error(
    'capture-mime-golden: utilsBundle.js.LICENSE names no mime version',
  )
  process.exit(2)
}
const { mime } = require(bundle)
const NAMES = [
  'a.png',
  'A.PNG',
  'photo.JPEG',
  'dir/a.txt',
  'dir\\a.json',
  'dir.d/noext',
  'noext',
  'txt',
  'dir/txt',
  '.bashrc',
  'x.',
  'archive.tar.gz',
  'a.b.c.html',
  'space name.txt',
  'a.unknownextension',
  '',
]
const types = Object.fromEntries(
  [...mime._getTestState().types].sort(([a], [b]) =>
    a < b ? -1 : a > b ? 1 : 0,
  ),
)
const getType = Object.fromEntries(
  NAMES.map((name) => [name, mime.getType(name)]),
)
const text = `${JSON.stringify({ playwright: version, mime: named[1], getType, types }, null, 2)}\n`

if (process.argv.includes('--check')) {
  const current = readFileSync(golden, 'utf8')
  if (current !== text) {
    console.error(
      'capture-mime-golden: tests/data/mime-golden.json is stale; rerun without --check',
    )
    process.exit(1)
  }
  console.log('capture-mime-golden: up to date')
} else {
  writeFileSync(golden, text)
  console.log(
    `capture-mime-golden: ${Object.keys(types).length} extensions from mime@${named[1]}`,
  )
}
