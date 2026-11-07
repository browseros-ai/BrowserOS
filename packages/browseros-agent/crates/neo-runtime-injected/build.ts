/**
 * Builds the page artifacts neo-runtime embeds with `include_str!`, plus the spec hash they were
 * built from:
 *
 *   dist/install.js     `(key, specSha256, initializer) => glue`   InjectedScript + glue
 *   dist/attach.js      `(key) => glue | undefined`                a few hundred bytes
 *   dist/call.js        `function (name, params, ...handles)`      the page-call convention
 *   dist/utility.js     `() => UtilityScript`                      Playwright's main-world helper
 *   dist/evaluate.js    `(utilityScript, ...args) => evaluate`     Playwright's evaluate convention
 *   dist/json-value.js  `(utilityScript, ...args) => jsonValue`    Playwright's jsonValue convention
 *   dist/get-property.js, wait-for-function.js, wait-result.js, wait-abort.js
 *                       Playwright's server page functions, run as `UtilityScript.evaluate`
 *                       expressions (`JSHandle.getProperty`, `page.waitForFunction`)
 *   dist/spec-sha256    neo-runtime-spec's SPEC_SHA256, no trailing newline, which neo-runtime
 *                       const-asserts against the Rust crate's own hash
 *
 * Each artifact is one function expression, because CDP's `Runtime.callFunctionOn` takes a
 * function declaration. Bun's ESM output is wrapped as
 * `(function (...args) { <bundle> return entry.apply(this, args) })`, much as neo-sdk's build.ts
 * wraps its bundle. That keeps every top-level name out of the world's globals and passes
 * `this` through for `call.js`, whose `this` is the glue object. There is no `'use strict'`:
 * Playwright evaluates its esbuild bundle as sloppy code (`server/dom.ts` wraps it in an arrow
 * function with no directive), and V8 rejects the directive in a function with a rest parameter.
 *
 * Bun, not esbuild: the repo's toolchain. A Bun plugin inlines `highlight.css?inline` as text,
 * as Playwright's esbuild plugin does (unminified, which changes no behaviour). Bun reads the
 * package tsconfig's `paths`, but it ignores `useDefineForClassFields: false` and always emits
 * class fields with define semantics, where Playwright's esbuild build assigns them. The package
 * tsconfig therefore type-checks the vendored code with `useDefineForClassFields: true`, under
 * which `tsc` rejects the two patterns whose behaviour differs between the semantics (a subclass
 * field that would reset a base-class property, TS2612; an initializer that reads a parameter
 * property, TS2729). `tests/load.test.mjs` proves the built bundle constructs `InjectedScript`
 * in a real page.
 *
 * Also enforced here, because Biome cannot tell a type import from a value import by path: no
 * file under `src/` except `install.ts` and `utility.ts` may value-import vendored code
 * (Playwright's DEPS.list rule: glue sees injected types, never injected code). `install.ts`
 * constructs `InjectedScript` for the utility world, `utility.ts` constructs `UtilityScript` for
 * the main world, and nothing else may build page code from the vendored tree. A violation fails
 * the build, and so neo-check.
 *
 * `scripts/neo-check.sh` runs this and fails when `dist/` differs from the committed files.
 */

import { readdirSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { SPEC_SHA256 } from '@browseros/neo-runtime-spec/spec-hash'
import type { BunPlugin } from 'bun'

const ENTRIES = [
  'install',
  'attach',
  'call',
  'utility',
  'evaluate',
  'json-value',
  'get-property',
  'wait-for-function',
  'wait-result',
  'wait-abort',
] as const
const VALUE_IMPORTERS = ['install.ts', 'utility.ts']
const VENDOR_ALIASES = ['@injected/', '@isomorphic/', '@protocol/']

const packageDir = import.meta.dir
// Bun writes each module's `// path` comment relative to the working directory, and a build
// started in packages/browseros-agent could not resolve this tsconfig's `paths`. From anywhere
// but here the build would fail or differ from the committed dist.
process.chdir(packageDir)
const srcDir = join(packageDir, 'src')
const vendorDir = join(packageDir, 'vendor')
const distDir = join(packageDir, 'dist')

/** Throws when a `src/` file other than `VALUE_IMPORTERS` value-imports vendored code. */
async function checkVendorImports(): Promise<void> {
  // scanImports drops `import type` and `export type`, and keeps an import whose names are used
  // only as types, so a vendored type must be imported with `import type` here.
  const transpiler = new Bun.Transpiler({ loader: 'ts' })
  const violations: string[] = []
  const files = readdirSync(srcDir, { recursive: true, encoding: 'utf8' })
  for (const file of files) {
    if (!file.endsWith('.ts') || file.endsWith('.d.ts')) continue
    if (VALUE_IMPORTERS.includes(file)) continue
    const path = join(srcDir, file)
    const imports = transpiler.scanImports(await Bun.file(path).text())
    for (const { path: specifier } of imports) {
      const vendored =
        VENDOR_ALIASES.some((alias) => specifier.startsWith(alias)) ||
        (specifier.startsWith('.') &&
          !relative(vendorDir, resolve(dirname(path), specifier)).startsWith(
            '..',
          ))
      if (vendored) violations.push(`src/${file}: ${specifier}`)
    }
  }
  if (violations.length > 0) {
    throw new Error(
      `only ${VALUE_IMPORTERS.map((file) => `src/${file}`).join(' and ')} may value-import vendored code; use \`import type\`:\n  ${violations.join('\n  ')}`,
    )
  }
}

/**
 * Playwright's `inlineCSSPlugin`: a `.css?inline` import is the stylesheet's text. The module's
 * path stays relative to the package, because Bun prints it into the bundle and an absolute path
 * would make dist/ differ between checkouts.
 */
const inlineCss: BunPlugin = {
  name: 'inline-css',
  setup(build) {
    build.onResolve({ filter: /\.css\?inline$/ }, (args) => ({
      path: relative(
        packageDir,
        resolve(dirname(args.importer), args.path.replace(/\?inline$/, '')),
      ),
      namespace: 'inline-css',
    }))
    build.onLoad({ filter: /.*/, namespace: 'inline-css' }, async (args) => ({
      contents: await Bun.file(join(packageDir, args.path)).text(),
      loader: 'text',
    }))
  },
}

async function buildEntry(name: (typeof ENTRIES)[number]): Promise<string> {
  const result = await Bun.build({
    entrypoints: [join(srcDir, `${name}.ts`)],
    format: 'esm',
    target: 'browser',
    plugins: [inlineCss],
  })
  const [output] = result.outputs
  if (!result.success || result.outputs.length !== 1 || !output) {
    throw new AggregateError(
      result.logs,
      `neo-runtime-injected: ${name} failed`,
    )
  }
  const esm = await output.text()
  const exportStatement = /^export\s*\{\s*(\w+)\s+as\s+default\s*\};?\s*$/m
  const entry = esm.match(exportStatement)?.[1]
  if (!entry || esm.match(/^export /gm)?.length !== 1) {
    throw new Error(
      `neo-runtime-injected: ${name} must have exactly one export, its default function`,
    )
  }
  const body = esm.replace(exportStatement, '').trimEnd()
  return `// Generated by build.ts from src/${name}.ts. Do not edit.\n(function (...args) {\n${body}\nreturn ${entry}.apply(this, args);\n})\n`
}

await checkVendorImports()
for (const name of ENTRIES) {
  await Bun.write(join(distDir, `${name}.js`), await buildEntry(name))
}
await Bun.write(join(distDir, 'spec-sha256'), SPEC_SHA256)
