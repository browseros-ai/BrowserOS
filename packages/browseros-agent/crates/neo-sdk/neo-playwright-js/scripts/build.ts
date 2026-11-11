/**
 * Builds every file neo-playwright embeds or tests against from this package and the
 * crate's vendored Playwright (third_party/playwright, never edited):
 *
 *   assets/injected.js   page side: src/probe/index.ts with Playwright's InjectedScript, one
 *                        IIFE whose only global is __neoProbe, evaluated in the isolated world
 *   assets/prelude.js    agent side: src/prelude/index.ts, the vendored isomorphic helpers, one
 *                        IIFE for QuickJS that defines no global (it hands the agent's
 *                        function its parameters through claw-sdk's `__neo.provide`)
 *   assets/main-world.js page side, main world: src/main-world/index.ts with Playwright's
 *                        UtilityScript, one function expression the evaluate guard sends as
 *                        the declaration of every agent script's `Runtime.callFunctionOn`
 *   assets/bindings-controller.js  page side, main world: Playwright's BindingsController,
 *                        one IIFE Rust registers as an init script where a run exposes a binding
 *   tests/data/probe-protocol.json   the typed protocol examples the Rust mirror decodes
 *   tests/data/in-page-values.json   Playwright's serialized values, what Rust writes back and
 *                                    their argument texts, read by Rust and the prelude's tests
 *                                    (scripts/in-page-values.ts)
 *   tests/data/wire-examples.json    every op input the fresh prelude sends, which the crate
 *                                    invokes offline (scripts/wire-examples.ts)
 *
 * `bun scripts/build.ts` writes them, and `--check` fails when a committed copy is stale. The
 * crate embeds the assets with include_str!, so Cargo needs no build step. Use Bun 1.4.2, the
 * repo's packageManager: other versions emit other bytes.
 */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, relative, resolve, sep } from 'node:path'
import type { BunPlugin } from 'bun'
import { protocolDocument } from '../src/probe/protocol-examples'
import { inPageValuesDocument } from './in-page-values'
import { wireExamplesDocument } from './wire-examples'

const packageRoot = resolve(import.meta.dir, '..')
const agentRoot = resolve(packageRoot, '../../..')
const crateRoot = resolve(agentRoot, 'crates/neo-sdk/neo-playwright')
const vendorRoot = resolve(crateRoot, 'third_party/playwright')

/** One generated file, its path relative to packages/browseros-agent. */
export type Output = { path: string; text: string }

// The inspector overlay and the window.playwright devtools helper sit in InjectedScript's
// import graph but are never used by Neo. Swapping them at resolution keeps every upstream
// file byte-identical, which sync.sh and the vendored-bytes check rely on.
const STUBS: Record<string, string> = {
  './highlight': resolve(packageRoot, 'src/stubs/highlight.ts'),
  './consoleApi': resolve(packageRoot, 'src/stubs/consoleApi.ts'),
}

/**
 * The stand-ins, and upstream's `@isomorphic/*` alias (its tsconfig) pointed at the vendored
 * copy. This package's own sources import vendored modules by relative path.
 */
const vendored: BunPlugin = {
  name: 'vendored-playwright',
  setup(build) {
    build.onResolve({ filter: /^\.\/(highlight|consoleApi)$/ }, (args) => {
      const stub = STUBS[args.path]
      if (!stub) throw new Error(`No stub for ${args.path}`)
      return { path: stub }
    })
    build.onResolve({ filter: /^@isomorphic\// }, (args) => ({
      path: resolve(
        vendorRoot,
        'packages/isomorphic',
        `${args.path.slice('@isomorphic/'.length)}.ts`,
      ),
    }))
  },
}

/** Playwright's value serializers, which the prelude runs in QuickJS (src/prelude/evaluation.ts). */
const VALUE_SERIALIZERS = new Set([
  resolve(vendorRoot, 'packages/isomorphic/utilityScriptSerializers.ts'),
  resolve(vendorRoot, 'packages/protocol/src/serializers.ts'),
])

/**
 * The prelude's bundle injects Neo's Node Buffer view (src/prelude/vendored-globals.ts) as a
 * module-scope import into the value serializers. The URL and base64 interfaces bind to
 * neo-sdk's real realm globals, installed before this prelude. The page's bundles load the
 * same files plain, on the page's own globals.
 */
const preludeGlobals: BunPlugin = {
  name: 'prelude-globals',
  setup(build) {
    build.onLoad(
      { filter: /(utilityScriptSerializers|protocol\/src\/serializers)\.ts$/ },
      async (args) => {
        if (!VALUE_SERIALIZERS.has(args.path))
          throw new Error(
            `${args.path} is not one of the vendored value serializers the prelude's stand-ins were reviewed on`,
          )
        const globals = resolve(packageRoot, 'src/prelude/vendored-globals.ts')
        const source = await Bun.file(args.path).text()
        return {
          contents: `import { Buffer } from ${JSON.stringify(globals)};\n${source}`,
          loader: 'ts',
        }
      },
    )
  },
}

function vendoredVersion(): { tag: string; commit: string } {
  const [tag = '', commit = ''] = readFileSync(
    resolve(vendorRoot, 'VERSION'),
    'utf8',
  )
    .trim()
    .split('\n')
  return { tag, commit }
}

async function bundle(
  entry: string,
  format: 'iife' | 'esm' = 'iife',
  plugins: BunPlugin[] = [vendored],
): Promise<string> {
  const buildCwd = process.cwd()
  const result = await Bun.build({
    entrypoints: [resolve(packageRoot, entry)],
    root: agentRoot,
    target: 'browser',
    format,
    minify: false,
    sourcemap: 'none',
    plugins,
  })
  if (!result.success)
    throw new AggregateError(result.logs, `Could not build ${entry}`)
  const [output] = result.outputs
  if (!output || result.outputs.length !== 1)
    throw new Error(`Expected exactly one bundle for ${entry}`)
  // Bun's module comments follow cwd even when root is set. The Rust tests read them to see
  // which modules a bundle holds, so they must not depend on where the build ran. An IIFE
  // indents them by two spaces; an ES module leaves them at the margin.
  const marker =
    format === 'iife' ? /^( {2})\/\/ (.+\.ts)$/gm : /^()\/\/ (.+\.ts)$/gm
  return (await output.text()).replace(
    marker,
    (_marker, indent: string, sourcePath: string) =>
      `${indent}// ${relative(agentRoot, resolve(buildCwd, sourcePath)).split(sep).join('/')}`,
  )
}

/**
 * src/main-world/index.ts as one function expression, the form `Runtime.callFunctionOn` takes
 * for its declaration. The module is bundled as ESM, its one default export is dropped, and its
 * code becomes the body of a function that calls that export with its own `this` and arguments.
 * The body is strict, as the module was. V8 compiles a declaration inside parentheses, so the
 * function ends in `}`: a line comment there would swallow the closing parenthesis.
 */
async function mainWorldFunction(): Promise<string> {
  const code = await bundle('src/main-world/index.ts', 'esm')
  const exported = /\nexport \{\n {2}(\w+) as default\n\};\n$/.exec(code)
  if (!exported)
    throw new Error(
      'src/main-world/index.ts must bundle to code that ends in its one default export',
    )
  const body = code.slice(0, exported.index)
  return `function () {\n'use strict';\n${body}\nreturn ${exported[1]}.apply(this, arguments);\n}`
}

export async function buildOutputs(): Promise<Output[]> {
  const { tag, commit } = vendoredVersion()
  const [injected, prelude, mainWorld, bindingsController] = await Promise.all([
    bundle('src/probe/index.ts'),
    bundle('src/prelude/index.ts', 'iife', [vendored, preludeGlobals]),
    mainWorldFunction(),
    bundle('src/main-world/bindings-controller.ts'),
  ])
  const preludeAsset = `/*
 * Playwright ${tag} (${commit}), packages/isomorphic helpers and packages/protocol value
 * serializers, for Neo's QuickJS prelude.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * mime: Copyright (c) 2023 Robert Kieffer. Licensed under the MIT License.
 * Built from crates/neo-sdk/neo-playwright/third_party/playwright and
 * crates/neo-sdk/neo-playwright/third_party/mime, with crates/neo-sdk/neo-playwright-js/src/prelude,
 * by crates/neo-sdk/neo-playwright-js/scripts/build.ts.
 * Changed by Neo: isomorphic/utilityScriptSerializers.ts and protocol/src/serializers.ts import
 * Neo's Buffer view, which QuickJS lacks; URL, atob and btoa use the realm globals.
 * Every other upstream module, and mime, is built unmodified.
 * Do not edit. Notices: crates/neo-sdk/neo-playwright/THIRD_PARTY_NOTICES.md.
 */
${prelude}`
  return [
    {
      path: 'crates/neo-sdk/neo-playwright/assets/injected.js',
      text: `/*
 * Neo's probe with Playwright ${tag} (${commit}), packages/injected InjectedScript.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * Built from crates/neo-sdk/neo-playwright/third_party/playwright, with
 * crates/neo-sdk/neo-playwright-js/src/probe, by crates/neo-sdk/neo-playwright-js/scripts/build.ts.
 * Every upstream module is built unmodified. Do not edit. Notices: crates/neo-sdk/neo-playwright/THIRD_PARTY_NOTICES.md.
 */
${injected}`,
    },
    {
      path: 'crates/neo-sdk/neo-playwright/assets/prelude.js',
      text: preludeAsset,
    },
    {
      path: 'crates/neo-sdk/neo-playwright/assets/main-world.js',
      text: `/*
 * Neo's main-world function with Playwright ${tag} (${commit}), packages/injected UtilityScript
 * and packages/isomorphic utilityScriptSerializers.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * Built unmodified from crates/neo-sdk/neo-playwright/third_party/playwright, with
 * crates/neo-sdk/neo-playwright-js/src/main-world, by crates/neo-sdk/neo-playwright-js/scripts/build.ts.
 * Do not edit. Notices: crates/neo-sdk/neo-playwright/THIRD_PARTY_NOTICES.md.
 */
${mainWorld}`,
    },
    {
      path: 'crates/neo-sdk/neo-playwright/assets/bindings-controller.js',
      text: `/*
 * Playwright ${tag} (${commit}), packages/injected BindingsController and packages/isomorphic
 * utilityScriptSerializers, installed in a page's main world for exposeBinding.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * Built unmodified from crates/neo-sdk/neo-playwright/third_party/playwright, with
 * crates/neo-sdk/neo-playwright-js/src/main-world/bindings-controller.ts, by
 * crates/neo-sdk/neo-playwright-js/scripts/build.ts.
 * Do not edit. Notices: crates/neo-sdk/neo-playwright/THIRD_PARTY_NOTICES.md.
 */
${bindingsController}`,
    },
    {
      path: 'crates/neo-sdk/neo-playwright/tests/data/probe-protocol.json',
      text: `${JSON.stringify(protocolDocument(), null, 2)}\n`,
    },
    {
      path: 'crates/neo-sdk/neo-playwright/tests/data/in-page-values.json',
      text: inPageValuesDocument(),
    },
    {
      // Recorded from this build's prelude, not the committed one, so one build is consistent.
      path: 'crates/neo-sdk/neo-playwright/tests/data/wire-examples.json',
      text: await wireExamplesDocument(preludeAsset),
    },
  ]
}

/** The outputs whose committed copy differs from a fresh build (or is missing). */
export async function staleOutputs(): Promise<string[]> {
  const stale: string[] = []
  for (const output of await buildOutputs()) {
    let committed: string | undefined
    try {
      committed = readFileSync(resolve(agentRoot, output.path), 'utf8')
    } catch {
      committed = undefined
    }
    if (committed !== output.text) stale.push(output.path)
  }
  return stale
}

if (import.meta.main) {
  if (process.argv.includes('--check')) {
    const stale = await staleOutputs()
    if (stale.length > 0) {
      console.error(
        `Stale: ${stale.join(', ')}. Run bun run build with Bun 1.4.2 in crates/neo-sdk/neo-playwright-js.`,
      )
      process.exit(1)
    }
  } else {
    for (const output of await buildOutputs()) {
      const target = resolve(agentRoot, output.path)
      mkdirSync(dirname(target), { recursive: true })
      writeFileSync(target, output.text)
    }
  }
}
