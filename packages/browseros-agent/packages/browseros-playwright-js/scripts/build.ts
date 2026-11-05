/**
 * Builds every file browseros-playwright embeds or tests against from this package and the
 * crate's vendored Playwright (third_party/playwright, never edited):
 *
 *   assets/injected.js   page side: src/probe/index.ts with Playwright's InjectedScript, one
 *                        IIFE whose only global is __neoProbe, evaluated in the sealed world
 *   assets/prelude.js    agent side: src/prelude/index.ts, the vendored isomorphic helpers, one
 *                        IIFE for QuickJS that defines no global (it hands the agent's
 *                        function its parameters through claw-sdk's `__neo.provide`)
 *   tests/data/probe-protocol.json   the typed protocol examples the Rust mirror decodes
 *   tests/data/wire-examples.json    every op input the fresh prelude sends, which the crate
 *                                    invokes offline (scripts/wire-examples.ts)
 *
 * `bun scripts/build.ts` writes them, and `--check` fails when a committed copy is stale. The
 * crate embeds the assets with include_str!, so Cargo needs no build step. Use Bun 1.4.2, the
 * repo's packageManager: other versions emit other bytes.
 *
 * The page asset puts Neo's mask on page text before the engine cuts or re-encodes it for a
 * preview or a locator hint (src/probe/page-mask.ts says why). The vendored files stay
 * byte-identical; the build does it at resolution and load:
 *
 *   @isomorphic/stringUtils from injectedScript.ts and selectorGenerator.ts
 *       resolves to src/probe/masked-strings.ts, upstream's module with its trim and quote
 *       helpers masking their input first
 *   selectorGenerator.ts
 *       loads with `maskPageText` called first in suitableTextAlternatives (its cuts at a word
 *       and a number) and escapeClassName (its CSS escapes), two functions of its own
 */
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, relative, resolve, sep } from 'node:path'
import type { BunPlugin } from 'bun'
import { protocolDocument } from '../src/probe/protocol-examples'
import { wireExamplesDocument } from './wire-examples'

const packageRoot = resolve(import.meta.dir, '..')
const agentRoot = resolve(packageRoot, '../..')
const crateRoot = resolve(agentRoot, 'crates/browseros-playwright')
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

const injectedSource = (file: string) =>
  resolve(vendorRoot, 'packages/injected/src', file)
const SELECTOR_GENERATOR = injectedSource('selectorGenerator.ts')

/** The engine files whose string helpers mask page text first (masked-strings.ts). */
const MASKED_STRING_IMPORTERS = new Set([
  injectedSource('injectedScript.ts'),
  SELECTOR_GENERATOR,
])

/**
 * The selector generator's own functions that cut or escape page text, each with the line it
 * opens with upstream. The build fails when one is not there exactly once, so an upgrade that
 * renames or reshapes one stops here until its mask is reviewed again.
 */
const MASKED_GENERATOR_FUNCTIONS = [
  {
    opening: 'function suitableTextAlternatives(text: string) {',
    input: 'text',
  },
  {
    opening: 'function escapeClassName(className: string): string {',
    input: 'className',
  },
]

/** selectorGenerator.ts with each of MASKED_GENERATOR_FUNCTIONS masking its input first. */
function maskedSelectorGenerator(source: string): string {
  let masked = source
  for (const { opening, input } of MASKED_GENERATOR_FUNCTIONS) {
    const [, ...rest] = masked.split(opening)
    if (rest.length !== 1)
      throw new Error(
        `selectorGenerator.ts opens \`${opening}\` ${rest.length} times, not once: review Neo's mask (src/probe/page-mask.ts)`,
      )
    masked = masked.replace(
      opening,
      `${opening}\n  ${input} = maskPageText(${input});`,
    )
  }
  const pageMask = resolve(packageRoot, 'src/probe/page-mask.ts')
  return `import { maskPageText } from ${JSON.stringify(pageMask)};\n${masked}`
}

/**
 * The stand-ins, Neo's mask on the engine's preview and hint text, and upstream's
 * `@isomorphic/*` alias (its tsconfig) pointed at the vendored copy. This package's own
 * sources import vendored modules by relative path.
 */
const vendored: BunPlugin = {
  name: 'vendored-playwright',
  setup(build) {
    build.onResolve({ filter: /^\.\/(highlight|consoleApi)$/ }, (args) => {
      const stub = STUBS[args.path]
      if (!stub) throw new Error(`No stub for ${args.path}`)
      return { path: stub }
    })
    build.onResolve({ filter: /^@isomorphic\/stringUtils$/ }, (args) =>
      MASKED_STRING_IMPORTERS.has(args.importer)
        ? { path: resolve(packageRoot, 'src/probe/masked-strings.ts') }
        : undefined,
    )
    build.onLoad({ filter: /selectorGenerator\.ts$/ }, async (args) => {
      if (args.path !== SELECTOR_GENERATOR)
        throw new Error(
          `${args.path} is not the vendored selector generator that Neo's mask was reviewed on`,
        )
      const source = await Bun.file(args.path).text()
      return { contents: maskedSelectorGenerator(source), loader: 'ts' }
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

function vendoredVersion(): { tag: string; commit: string } {
  const [tag = '', commit = ''] = readFileSync(
    resolve(vendorRoot, 'VERSION'),
    'utf8',
  )
    .trim()
    .split('\n')
  return { tag, commit }
}

async function bundle(entry: string): Promise<string> {
  const buildCwd = process.cwd()
  const result = await Bun.build({
    entrypoints: [resolve(packageRoot, entry)],
    root: agentRoot,
    target: 'browser',
    format: 'iife',
    minify: false,
    sourcemap: 'none',
    plugins: [vendored],
  })
  if (!result.success)
    throw new AggregateError(result.logs, `Could not build ${entry}`)
  const [output] = result.outputs
  if (!output || result.outputs.length !== 1)
    throw new Error(`Expected exactly one bundle for ${entry}`)
  // Bun's module comments follow cwd even when root is set. The Rust tests read them to see
  // which modules a bundle holds, so they must not depend on where the build ran.
  return (await output.text()).replace(
    /^( {2})\/\/ (.+\.ts)$/gm,
    (_marker, indent: string, sourcePath: string) =>
      `${indent}// ${relative(agentRoot, resolve(buildCwd, sourcePath)).split(sep).join('/')}`,
  )
}

export async function buildOutputs(): Promise<Output[]> {
  const { tag, commit } = vendoredVersion()
  const [injected, prelude] = await Promise.all([
    bundle('src/probe/index.ts'),
    bundle('src/prelude/index.ts'),
  ])
  const preludeAsset = `/*
 * Playwright ${tag} (${commit}), packages/isomorphic helpers, for Neo's QuickJS prelude.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * Built unmodified from crates/browseros-playwright/third_party/playwright, with
 * packages/browseros-playwright-js/src/prelude, by packages/browseros-playwright-js/scripts/build.ts.
 * Do not edit. Notices: crates/browseros-playwright/THIRD_PARTY_NOTICES.md.
 */
${prelude}`
  return [
    {
      path: 'crates/browseros-playwright/assets/injected.js',
      text: `/*
 * Neo's probe with Playwright ${tag} (${commit}), packages/injected InjectedScript.
 * Playwright: Copyright (c) Microsoft Corporation. Licensed under the Apache License, Version 2.0.
 * Built from crates/browseros-playwright/third_party/playwright, with
 * packages/browseros-playwright-js/src/probe, by packages/browseros-playwright-js/scripts/build.ts.
 * Changed by Neo: selectorGenerator.ts's suitableTextAlternatives and escapeClassName mask
 * their input first (Neo's maskPageText), and injectedScript.ts and selectorGenerator.ts import
 * Neo's masked stringUtils helpers; every other upstream module is built unmodified.
 * Do not edit. Notices: crates/browseros-playwright/THIRD_PARTY_NOTICES.md.
 */
${injected}`,
    },
    {
      path: 'crates/browseros-playwright/assets/prelude.js',
      text: preludeAsset,
    },
    {
      path: 'crates/browseros-playwright/tests/data/probe-protocol.json',
      text: `${JSON.stringify(protocolDocument(), null, 2)}\n`,
    },
    {
      // Recorded from this build's prelude, not the committed one, so one build is consistent.
      path: 'crates/browseros-playwright/tests/data/wire-examples.json',
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
        `Stale: ${stale.join(', ')}. Run bun run build with Bun 1.4.2 in packages/browseros-playwright-js.`,
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
