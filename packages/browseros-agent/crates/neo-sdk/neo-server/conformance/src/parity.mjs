/** Compare our registration to the real pinned Playwright Test loader. Only fixture providers
 * and Node-only internal bundles are substituted in a private copy; pageTest/specs stay verbatim. */

import { execFileSync } from 'node:child_process'
import {
  cpSync,
  existsSync,
  mkdirSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'

const require = createRequire(import.meta.url)
export function parity(catalog, pin, directory) {
  mkdirSync(directory, { recursive: true })
  writeFileSync(
    join(directory, 'package.json'),
    JSON.stringify({ type: 'commonjs' }),
  )
  cpSync(join(pin.dir, 'tests'), join(directory, 'tests'), { recursive: true })
  const pwRoot = dirname(require.resolve('playwright/package.json')),
    coreRoot = dirname(require.resolve('playwright-core/package.json'))
  const modules = resolve(directory, 'node_modules')
  mkdirSync(modules, { recursive: true })
  mkdirSync(join(modules, '@playwright'), { recursive: true })
  for (const [name, target] of [
    ['playwright', pwRoot],
    ['playwright-core', coreRoot],
    [
      '@playwright/test',
      dirname(require.resolve('@playwright/test/package.json')),
    ],
  ])
    if (!existsSync(join(modules, name)))
      symlinkSync(target, join(modules, name))
  const workerNames = [
    'server',
    'httpsServer',
    'asset',
    'loopback',
    'isAndroid',
    'isBidi',
    'isElectron',
    'isFrozenWebkit',
    'isHeadlessShell',
    'isWebView2',
    'isLinux',
    'isMac',
    'isWindows',
    'macVersion',
    'mode',
    'nodeVersion',
    'platform',
    'browserVersion',
    'browserMajorVersion',
    'electronMajorVersion',
    'toImpl',
    'contextFactory',
    'launchPersistent',
    'browserType',
    'allowsThirdParty',
    'defaultSameSiteCookieValue',
  ]
  const fixtureValues = {
    ...catalog.fixtures,
    asset: null,
    nodeVersion: { major: 24 },
    browserMajorVersion: 155,
  }
  writeFileSync(
    join(directory, 'tests/config/browserTest.ts'),
    `import { test as base } from '@playwright/test';\nexport const browserTest = base.extend({${workerNames.map((name) => `${name}: [async ({}, use) => use(${JSON.stringify(fixtureValues[name] ?? null)}), {scope:'worker'}]`).join(',')}});\n`,
  )
  for (const [dir, name] of [
    ['android', 'androidTest'],
    ['electron', 'electronTest'],
    ['webview', 'webviewTest'],
  ]) {
    mkdirSync(join(directory, 'tests', dir), { recursive: true })
    writeFileSync(
      join(directory, 'tests', dir, `${name}.ts`),
      `export {browserTest as ${name}} from '../config/browserTest';`,
    )
  }
  for (const [name, target] of [
    ['coreBundle', join(coreRoot, 'lib/coreBundle.js')],
    ['utilsBundle', join(coreRoot, 'lib/utilsBundle.js')],
  ]) {
    const dest = join(directory, 'packages/playwright-core/lib')
    mkdirSync(dest, { recursive: true })
    writeFileSync(
      join(dest, `${name}.js`),
      `module.exports = require(${JSON.stringify(target)});`,
    )
  }
  mkdirSync(join(modules, 'formidable'), { recursive: true })
  writeFileSync(join(modules, 'formidable/index.js'), 'module.exports = {};')
  writeFileSync(
    join(directory, 'playwright.config.cjs'),
    "module.exports={testDir:'./tests/page',workers:1};",
  )
  let output
  try {
    output = execFileSync(
      process.env.CONFORMANCE_NODE || 'node',
      [
        join(pwRoot, 'cli.js'),
        'test',
        '--list',
        '--reporter=json',
        '--config',
        join(directory, 'playwright.config.cjs'),
      ],
      { encoding: 'utf8', maxBuffer: 32 * 1024 * 1024, timeout: 60000 },
    )
  } catch (error) {
    writeFileSync(join(directory, 'list-error.json'), error.stdout || '')
    throw new Error('Playwright registration failed; see list-error.json')
  }
  writeFileSync(join(directory, 'list.json'), output)
  const listed = [],
    walk = (suite, titles = []) => {
      const path =
        suite.line === 0 || !suite.title ? titles : [...titles, suite.title]
      for (const spec of suite.specs || [])
        listed.push({
          spec: `tests/page/${spec.file.split('/').pop()}`,
          titlePath: [...path, spec.title],
          line: spec.line,
        })
      for (const child of suite.suites || []) walk(child, path)
    }
  for (const suite of JSON.parse(output).suites) walk(suite)
  return compareRegistration(catalog.inventory.tests, listed)
}

// The standalone screenshot suite covers this Node-only spec. Count its registrations
// explicitly so a new omission cannot hide behind the exception.
const screenshotSpec = 'tests/page/page-screenshot.spec.ts'
function compareRegistration(rows, listed) {
  const key = ({ spec, titlePath, line }) =>
    JSON.stringify([spec, titlePath, line])
  const registered = rows.filter((row) => row.spec.startsWith('tests/page/'))
  const exceptions = listed.filter((row) => row.spec === screenshotSpec)
  if (exceptions.length !== 67)
    throw new Error('upstream screenshot registration exception changed')
  const identities = new Map()
  for (const row of listed) {
    if (row.spec === screenshotSpec) continue
    const id = key(row)
    identities.set(id, (identities.get(id) || 0) + 1)
  }
  const unexpected = []
  for (const row of registered) {
    const id = key(row)
    const remaining = identities.get(id) || 0
    if (remaining === 0) unexpected.push(id)
    else if (remaining === 1) identities.delete(id)
    else identities.set(id, remaining - 1)
  }
  if (unexpected.length || identities.size)
    throw new Error(
      `registration mismatch: missing=${JSON.stringify([...identities.keys()].slice(0, 3))} unexpected=${JSON.stringify(unexpected.slice(0, 3))}`,
    )
  return {
    registered: registered.length,
    upstream: listed.length,
    unmatched: 0,
  }
}
