/** Loads complete upstream modules with Playwright's own transform. Spec functions, hooks and
 * closure state stay together; only Test infrastructure is substituted by the common harness. */
import { createHash } from 'node:crypto'
import { readdirSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import os from 'node:os'
import { dirname, join, posix, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import vm from 'node:vm'

const here = dirname(fileURLToPath(import.meta.url))
const req = createRequire(import.meta.url)
const nodeModules = dirname(dirname(req.resolve('playwright/package.json')))
export function load(upstreamDir, pin, extraDir) {
  const pwVersion = req('playwright/package.json').version
  const commit = pin.commit
  const upstreamVersion = JSON.parse(
    readFileSync(join(upstreamDir, 'package.json'), 'utf8'),
  ).version
  if (pwVersion !== upstreamVersion)
    throw new Error(`playwright ${pwVersion} != upstream ${upstreamVersion}`)
  process.env.BROWSERSLIST_IGNORE_OLD_DATA = 'true'
  const { babelTransform } = req(
    resolve(nodeModules, 'playwright/lib/transform/babelBundle.js'),
  )
  const harnessSource = readFileSync(join(here, 'runtime.js'), 'utf8')
  // Formatters may insert an ASI guard before this expression. Embed the parsed function,
  // not the statement's leading semicolon, so every program remains valid in both engines.
  const harnessFunctionSource = vm.runInThisContext(harnessSource).toString()

  // ---- compile: Playwright Test's own transform, CommonJS, as its loader runs these specs ------
  const sourcePath = (rel) =>
    rel.startsWith('selftest/') || rel.startsWith('screenshots/')
      ? join(extraDir, rel)
      : join(upstreamDir, rel)
  const compiled = new Map()
  function compile(rel) {
    if (compiled.has(rel)) return compiled.get(rel)
    const code = readFileSync(sourcePath(rel), 'utf8')
    const result = babelTransform(
      code,
      join('/upstream', rel),
      false,
      [[join(here, 'diagnostic-plugin.cjs'), {}]],
      undefined,
    )
    const messageAssertions = result.metadata.messageAssertions || []
    const body = result.code.replace(/\n\/\/# sourceMappingURL=.*$/s, '')
    const entry = {
      rel,
      body,
      map: result.map,
      sha256: createHash('sha256').update(code).digest('hex'),
      messageAssertions,
    }
    compiled.set(rel, entry)
    return entry
  }

  // ---- module map: which specifier each runtime gets -------------------------------------------
  const NODE_BUILTINS = new Set([
    'path',
    'fs',
    'os',
    'util',
    'url',
    'crypto',
    'child_process',
    'http',
    'https',
    'zlib',
    'stream',
    'net',
    'events',
    'assert',
    'worker_threads',
  ])
  /** The module a specifier names from `fromRel`, as the program's loader resolves it. */
  function resolveSpecifier(fromRel, spec) {
    if (spec === '@playwright/test') return { kind: 'harness-test' }
    if (spec.startsWith('.')) {
      const rel = posix.normalize(posix.join(posix.dirname(fromRel), spec))
      if (
        rel === 'tests/config/browserTest' ||
        /tests\/(android|electron|webview)\//.test(rel)
      )
        return { kind: 'harness-test' }
      if (rel.startsWith('packages/'))
        return { kind: 'unavailable', what: `upstream internal module ${rel}` }
      for (const ext of ['', '.ts', '.js', '/index.ts']) {
        try {
          readFileSync(sourcePath(rel + ext))
          return { kind: 'source', rel: rel + ext }
        } catch {}
      }
      return { kind: 'unavailable', what: `missing module ${rel}` }
    }
    if (NODE_BUILTINS.has(spec.replace(/^node:/, '')))
      return { kind: 'unavailable', what: `Node's ${spec}` }
    return { kind: 'unavailable', what: `npm package ${spec}` }
  }

  function unavailableModule(what, HarnessUnavailable) {
    // Loads fine (a type-only or unused import costs nothing); any use fails as the harness's.
    return new Proxy(
      {},
      {
        get: (_, name) =>
          name === '__esModule'
            ? false
            : name === 'default'
              ? unavailableModule(what, HarnessUnavailable)
              : () => {
                  throw new HarnessUnavailable(`${what} (${String(name)})`)
                },
      },
    )
  }

  // ---- source-map line lookup (VLQ), so a registration names its upstream line ----------------
  function decodeLines(map) {
    const B64 =
      'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
    const lines = []
    let srcLine = 0
    for (const group of map.mappings.split(';')) {
      const segs = []
      let col = 0
      for (const seg of group.split(',')) {
        if (!seg) continue
        const vals = []
        let v = 0,
          shift = 0
        for (const ch of seg) {
          const d = B64.indexOf(ch)
          v += (d & 31) << shift
          if (d & 32) shift += 5
          else {
            vals.push(v & 1 ? -(v >> 1) : v >> 1)
            v = 0
            shift = 0
          }
        }
        col += vals[0]
        if (vals.length >= 4) {
          srcLine += vals[2]
          segs.push([col, srcLine + 1])
        }
      }
      lines.push(segs)
    }
    return lines
  }

  // ---- registration: run one spec's top level in Node under the registrar ----------------------
  const CONSTANT_FIXTURES = {
    browserName: 'chromium',
    channel: undefined,
    mode: 'default',
    headless: true,
    isAndroid: false,
    isElectron: false,
    isBidi: false,
    isWebView2: false,
    isFrozenWebkit: false,
    isHeadlessShell: false,
    isMac: os.platform() === 'darwin',
    isWindows: os.platform() === 'win32',
    isLinux: os.platform() === 'linux',
    // Upstream's platformFixtures.ts and browserTest.ts definitions, true of the one host both runners share.
    platform: os.platform(),
    electronMajorVersion: 0,
    trace: 'off',
    video: 'off',
    screenshot: 'off',
    macVersion:
      os.platform() !== 'darwin'
        ? 0
        : +os.release().split('.')[0] >= 25
          ? +os.release().split('.')[0] + 1
          : +os.release().split('.')[0] - 9,
    nodeVersion: Object.fromEntries(
      ['major', 'minor', 'patch'].map((k, i) => [
        k,
        +process.versions.node.split('.')[i],
      ]),
    ),
  }
  // Read from the browser when a run starts (/json/version), identical for both runners' one binary.
  const RUNTIME_FIXTURES = ['browserVersion', 'browserMajorVersion']
  const PROVIDED = new Set([
    'page',
    'context',
    'server',
    'asset',
    ...Object.keys(CONSTANT_FIXTURES),
    ...RUNTIME_FIXTURES,
  ])

  function loadSpec(specRel) {
    const _filenames = new Map()
    const registrationExpect = new Proxy(() => registrationExpect, {
      get: (_, name) => (name === 'then' ? undefined : registrationExpect),
    })
    const harness = vm.runInThisContext(harnessSource)({
      config: { fixtures: CONSTANT_FIXTURES, server: {}, assetDir: '' },
      expect: registrationExpect, // registration only: statics such as expect.objectContaining are callable
      locate: (stack) => {
        for (const line of stack.split('\n')) {
          const m =
            /\((\/upstream\/[^:]+):(\d+):(\d+)\)|at (\/upstream\/[^:]+):(\d+):(\d+)/.exec(
              line,
            )
          if (!m) continue
          const [file, ln, col] = m[1]
            ? [m[1], +m[2], +m[3]]
            : [m[4], +m[5], +m[6]]
          const rel = file.slice('/upstream/'.length)
          const entry = compiled.get(rel)
          if (
            !entry ||
            !(
              rel.startsWith('tests/page/') ||
              rel.startsWith('selftest/') ||
              rel.startsWith('screenshots/')
            ) ||
            rel.endsWith('pageTest.ts')
          )
            continue
          entry.lines ??= decodeLines(entry.map)
          // The wrapper adds one line in front of each module body.
          const segs = entry.lines[ln - 2] || []
          let best = null
          for (const [c, srcLine] of segs) if (c <= col - 1) best = srcLine
          return { file: rel, line: best ?? segs[0]?.[1] ?? null }
        }
        return null
      },
    })
    const unavailable = new Set()
    const cache = new Map()
    const load = (rel) => {
      if (cache.has(rel)) return cache.get(rel).exports
      const entry = compile(rel)
      const module = { exports: {} }
      cache.set(rel, module)
      const fn = vm.runInThisContext(
        `(function (exports, require, module, __filename, __dirname, process, __harness) {\n${entry.body}\n})`,
        { filename: join('/upstream', rel) },
      )
      const requireFrom = (spec) => {
        const target = resolveSpecifier(rel, spec)
        if (target.kind === 'harness-test')
          return {
            test: harness.test,
            expect: harness.expect,
            browserTest: harness.test,
            androidTest: harness.test,
            electronTest: harness.test,
            webviewTest: harness.test,
          }
        if (target.kind === 'source') return load(target.rel)
        unavailable.add(target.what)
        return unavailableModule(target.what, harness.HarnessUnavailable)
      }
      fn(
        module.exports,
        requireFrom,
        module,
        join('/upstream', rel),
        join('/upstream', posix.dirname(rel)),
        { env: {}, platform: os.platform() },
        harness,
      )
      return module.exports
    }
    let loadError = null
    try {
      load(specRel)
    } catch (error) {
      loadError = `${error.name}: ${error.message}`
    }
    return { harness, unavailable: [...unavailable].sort(), loadError }
  }

  // ---- static classification: what is decided before anything runs ----------------------------
  const INTERNAL_TOKENS = [
    '__testHook',
    'window.builtins',
    'toImpl',
    '_page._delegate',
  ]
  function classify(harness, test) {
    const chain = harness.chain(test.suite)
    const pre = [...chain.flatMap((s) => s.modifiers), ...test.modifiers].find(
      (m) => (m.kind === 'skip' || m.kind === 'fixme') && m.condition === true,
    )
    if (pre && (pre.kind === 'skip' || pre.kind === 'fixme'))
      return {
        class: 'upstream-skip',
        reason: `test.${pre.kind}${pre.description ? `: ${pre.description}` : ''}`,
      }
    const uses = chain.flatMap((s) => s.use.map((u) => Object.keys(u.options)))
    if (uses.length)
      return {
        class: 'excluded',
        kind: 'context-options',
        reason: `test.use({ ${[...new Set(uses.flat())].join(', ')} }) configures a context Neo cannot reconfigure`,
      }
    if (chain.some((s) => s.mode === 'serial'))
      return {
        class: 'excluded',
        kind: 'serial',
        reason: 'a serial describe: its tests share state across tests',
      }
    const hookFns = chain.flatMap((s) =>
      [
        ...s.hooks.beforeAll,
        ...s.hooks.beforeEach,
        ...s.hooks.afterEach,
        ...s.hooks.afterAll,
      ].map((h) => h.fn),
    )
    const fixtures = new Set(
      [test.fn, ...hookFns].flatMap((fn) => harness.fixtureNames(fn)),
    )
    const extended = [...fixtures].filter((f) => f in test.fixtures)
    if (extended.length)
      return {
        class: 'excluded',
        kind: 'extend-fixture',
        reason: `uses test.extend fixtures: ${extended.join(', ')}`,
      }
    const missing = [...fixtures].filter((f) => !PROVIDED.has(f))
    if (missing.length)
      return {
        class: 'excluded',
        kind: 'fixture',
        reason: `uses fixtures the harness does not provide: ${missing.join(', ')}`,
      }
    const text = test.fn.toString()
    const internal = INTERNAL_TOKENS.filter((t) => text.includes(t))
    if (internal.length)
      return {
        class: 'excluded',
        kind: 'internal',
        reason: `uses Playwright's internal test hooks: ${internal.join(', ')}`,
      }
    const runtimeModifiers = /\b(it|test)\.(skip|fixme|fail|slow)\(/.test(text)
    return {
      class: 'runnable',
      note: runtimeModifiers ? 'decides a modifier at runtime' : undefined,
      fixtures: [...fixtures].sort(),
    }
  }

  // ---- main ------------------------------------------------------------------------------------
  const coreSpec = (name) =>
    [
      /^selectors-(role|text|css|get-by)\.spec\.ts$/,
      /^locator-.+\.spec\.ts$/,
      /^page-click.*\.spec\.ts$/,
      /^page-(fill|check|keyboard|set-input-files)\.spec\.ts$/,
      /^page-wait-for-selector.*\.spec\.ts$/,
      /^page-aria-snapshot-ai\.spec\.ts$/,
    ].some((re) => re.test(name))
  // These pinned upstream cases require infrastructure outside the shared runner.
  // Keep their registrations and exact reasons visible, including transitive rafraf use.
  const unavailableCases = new Map(
    Object.entries(
      JSON.parse(
        readFileSync(join(extraDir, 'harness-unavailable.json'), 'utf8'),
      ),
    ),
  )
  const specs = readdirSync(join(upstreamDir, 'tests/page'))
    .filter((f) => f.endsWith('.spec.ts'))
    .sort()
    .map((f) => `tests/page/${f}`)
  for (const dir of ['selftest', 'screenshots']) {
    try {
      for (const f of readdirSync(join(extraDir, dir))
        .filter((f) => f.endsWith('.spec.ts'))
        .sort())
        specs.push(`${dir}/${f}`)
    } catch {}
  }
  const rows = [],
    specRows = []
  for (const rel of specs) {
    const { harness, unavailable, loadError } = loadSpec(rel)
    specRows.push({
      spec: rel,
      sha256: compile(rel).sha256,
      tests: harness.tests.length,
      loadError,
      unavailableImports: unavailable,
      messageAssertions: compile(rel).messageAssertions,
    })
    harness.tests.forEach((test, ordinal) => {
      const titlePath = harness.titlePath(test)
      const id = `${rel} › ${titlePath.join(' › ')}`
      const unavailable = unavailableCases.get(id)
      unavailableCases.delete(id)
      rows.push({
        id,
        spec: rel,
        ordinal,
        line: test.location?.line ?? null,
        titlePath,
        selected:
          !rel.startsWith('tests/page/') || coreSpec(rel.split('/').pop()),
        ...(unavailable
          ? {
              class: 'excluded',
              kind: 'harness-unavailable',
              reason: unavailable,
            }
          : classify(harness, test)),
      })
    })
  }
  if (unavailableCases.size)
    throw new Error(
      `unknown harness limitation: ${[...unavailableCases.keys()].join(', ')}`,
    )
  const inventory = {
    upstream: {
      commit,
      version: upstreamVersion,
      transform: `playwright@${pwVersion} babelTransform`,
    },
    specs: specRows,
    tests: rows,
  }
  // ---- programs: the exact bytes both runners evaluate for one test ----------------------------
  function program(specRel, ordinal, config) {
    const modules = new Map()
    const visit = (rel) => {
      if (modules.has(rel)) return
      const entry = compile(rel)
      const deps = {}
      modules.set(rel, { entry, deps })
      for (const m of entry.body.matchAll(/require\("([^"]+)"\)/g)) {
        const target = resolveSpecifier(rel, m[1])
        deps[m[1]] = target
        if (target.kind === 'source') visit(target.rel)
      }
    }
    visit('tests/page/pageTest.ts')
    visit(specRel)
    const test = rows.find((r) => r.spec === specRel && r.ordinal === ordinal)
    const parts = [
      `// Upstream Playwright ${upstreamVersion} (${commit}) ${specRel}:${test.line}`,
      `// Test: ${test.titlePath.join(' > ')}`,
      `const __config = ${JSON.stringify(config)}`,
      `const __target = ${JSON.stringify({ spec: specRel, ordinal, titlePath: test.titlePath })}`,
      `const __harness = (${harnessFunctionSource})({ context, expect, setTimeout, clearTimeout, now: () => Date.now(), config: __config, phase: typeof __conformancePhase === 'function' ? __conformancePhase : undefined })`,
      'const __modules = {',
      ...[...modules].map(
        ([rel, { entry, deps }]) =>
          `${JSON.stringify(rel)}: { deps: ${JSON.stringify(deps)}, fn: function (exports, require, module, __filename, __dirname, process, __harness) {\n${entry.body}\n} },`,
      ),
      '}',
      `const __cache = {}
function __unavailable(what) { return new Proxy({}, { get: (_, n) => n === '__esModule' ? false : n === 'default' ? __unavailable(what) : (() => { throw new __harness.HarnessUnavailable(what + ' (' + String(n) + ')') }) }) }
function __load(rel) {
  if (__cache[rel]) return __cache[rel].exports
  const { deps, fn } = __modules[rel]
  const module = (__cache[rel] = { exports: {} })
  const require = (spec) => {
    const target = deps[spec]
    if (target.kind === 'harness-test') return { test: __harness.test, expect: __harness.expect, browserTest: __harness.test, androidTest: __harness.test, electronTest: __harness.test, webviewTest: __harness.test }
    if (target.kind === 'source') return __load(target.rel)
    return __unavailable(target.what)
  }
  fn(module.exports, require, module, '/upstream/' + rel, '/upstream/' + rel.replace(/\\/[^/]*$/, ''), { env: {}, platform: __config.platform }, __harness)
  return module.exports
}
__load(__target.spec)
const __test = __harness.tests[__target.ordinal]
if (!__test || JSON.stringify(__harness.titlePath(__test)) !== JSON.stringify(__target.titlePath))
  return { harnessError: 'the program registered a different test at this ordinal than the inventory lists' }
const __record = await __harness.runOne(__target.ordinal)
__record.id = ${JSON.stringify(test.id)}
__record.ordinal = __target.ordinal
return __record`,
    ]
    return parts.join('\n')
  }
  return { inventory, program, fixtures: CONSTANT_FIXTURES }
}
