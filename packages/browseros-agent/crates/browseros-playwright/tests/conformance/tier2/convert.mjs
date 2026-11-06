/**
 * Conformance tier 2 (design D15): lifts upstream Playwright's own page tests into tier-1 case
 * files (harness.js is the contract), so the suite grows from upstream's tests instead of by
 * hand. The input is the copy pinned under third_party/playwright/tests/ at the vendored tag
 * (sync.sh copies it, SHA256SUMS pins it, and this script checks every file it reads against
 * SHA256SUMS), so a rerun anywhere gives the same bytes.
 *
 *   PLAYWRIGHT_NODE_MODULES=<node_modules with playwright at the vendored version> \
 *   node tier2/convert.mjs [--check]
 *
 * It writes cases/tier2/<spec>.<title>.js for every test it lifts and tier2/skipped.json for
 * every test it does not, with the reason. --check writes nothing and exits 1 when any of those
 * files differs from what it would write. Babel comes from the `playwright` package at the
 * vendored version, the parser upstream's own runner loads these specs with.
 *
 * A test is lifted when it uses only `page` (and `context`), upstream's static test-server
 * assets and asset files, constant fixtures such as `browserName`, helpers from its spec or from
 * tests/config/utils.ts, and members Neo has. What the lift rewrites, each in place:
 *
 *   x.setContent(html, o)   x.goto(<data URL of html>, o): Runner P cannot setContent on
 *                           BrowserOS (decisions.md), and both runners must open the same page
 *   expect(v).toBe(..)      tier2.record('L<line>', () => v): the received value is recorded
 *                           under the expect's upstream line, and Runner P is the oracle for it
 *   expect(v).toMatch(/re/) tier2.record('L<line>', () => tier2.matches(v, /re/)): upstream's
 *                           pattern is what it accepts, so its verdict is what is compared
 *   expect(v).toBeLessThan(n)
 *                           tier2.record('L<line>', () => (v) < (n)): so is its bound, and the
 *                           same for <=, > and >= (a count of scroll events, say)
 *   await expect(l).toX()   tier2.record('L<line>', () => expect(l).toX()): a web-first
 *                           assertion is kept and its outcome recorded (tier2.pass where its
 *                           promise is used, so a rejection still reaches the test)
 *   server.PREFIX ...       upstream's assets, served by server.mjs under /upstream/
 *   asset(name)             the vendored file, by a path relative to the conformance directory
 *                           both runners work in (upstream's is absolute), so a path a script
 *                           passes names the same file on both
 *   TypeScript              types removed in place, so a case reads like its spec
 *   window.builtins         window: Playwright installs builtins only under its own test mode
 *                           (utilityScript.ts isUnderTest), and nothing here emulates a clock
 *
 * A member Neo has but has not built yet, or a page Neo cannot run a test on yet (SWEEP_COST),
 * gives the case a `// @needs <unit>` line naming the unit that builds it (UNITS), so both
 * runners skip it until that unit lands. When it lands, drop its members from memberStatus
 * (and web-first matchers from `assertion`, or its tests from SWEEP_COST) and rerun: the
 * cases lose their `@needs` line and both runners run them. A recorded value is shaped as JSON
 * by the runtime at the end of each case: handles and locators become `(Locator)` and the
 * like, and messages lose their call log and stack frames, which are each runner's own.
 */
import { createHash } from 'node:crypto'
import {
  mkdirSync,
  readdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { basename, dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const upstream = resolve(here, '../../../third_party/playwright')
const casesDir = resolve(here, '../cases/tier2')
const skippedFile = resolve(here, 'skipped.json')

const SPECS = [
  /^selectors-(role|text|css|get-by)\.spec\.ts$/,
  /^locator-.+\.spec\.ts$/,
  /^page-click.*\.spec\.ts$/,
  /^page-(fill|check|keyboard|set-input-files)\.spec\.ts$/,
  /^page-wait-for-selector.*\.spec\.ts$/,
  /^page-aria-snapshot-ai\.spec\.ts$/,
]
const PAGE_TEST = 'tests/page/pageTest.ts'
const UTILS = 'tests/config/utils.ts'

// ---- the domain: what each upstream name means to Neo ------------------------------------

/**
 * The units building what a lifted test needs and Neo has not built, as `@needs` names them:
 * a member (memberStatus), or the cost a test's page asks for (SWEEP_COST).
 */
const UNITS = {
  'sweep-cost':
    "the privacy sweep's cost on big pages (m1-handoff's owner question)",
  unowned: 'no unit builds this yet',
}

/**
 * Tests Neo runs, but not on their page within a case's budget, by spec and title: they
 * build 24,575 elements and run main-world scripts over them. Measured on BrowserOS neo under
 * the delegates' load (load average 40 to 55): each op's sweep of such a page takes about
 * FRAME_BUDGET (1 s) and fails the op closed past it, one `$$eval` of one match took 3.2 s, and
 * the selectors-css case ran past Runner N's 90 s, while Runner P finished both in the same
 * runs. Drop an entry once its case fits, and the unit once none is left.
 */
const SWEEP_COST = new Map([
  [
    'tests/page/selectors-css.spec.ts > should work with large DOM @smoke',
    'cheap sweeps of a 24,575-element page',
  ],
  [
    'tests/page/selectors-text.spec.ts > should work with large DOM',
    'cheap sweeps of a 24,575-element page',
  ],
])

/**
 * Upstream's worker fixtures whose value is fixed for these runs: Chromium, not Android,
 * Electron, WebView2 or BiDi, default mode, no channel. A skip condition on any other fixture
 * (headless, the platform) cannot be decided here, so its test is skipped with that reason.
 */
const FIXTURE_VALUES = {
  browserName: 'chromium',
  channel: undefined,
  isAndroid: false,
  isBidi: false,
  isElectron: false,
  isFrozenWebkit: false,
  isWebView2: false,
  mode: 'default',
}
/** Fixtures a case defines itself, by the line that defines each, beside harness.js's own. */
const CASE_FIXTURES = {
  asset:
    "const asset = (name) => '../../third_party/playwright/tests/assets/' + name",
  server: 'const server = tier2.server',
}
const PROVIDED = new Set(['page', 'context', ...Object.keys(CASE_FIXTURES)])
const SERVER_MEMBERS = new Set(['PREFIX', 'CROSS_PROCESS_PREFIX', 'EMPTY_PAGE'])

/** Members whose function argument (at this index) runs in the page, not in the runner. */
const PAGE_FUNCTION_ARG = new Map([
  ['evaluate', 0],
  ['evaluateAll', 0],
  ['evaluateHandle', 0],
  ['waitForFunction', 0],
  ['addInitScript', 0],
  ['$eval', 1],
  ['$$eval', 1],
])
/** Members Neo refuses (packages/browseros-playwright-js/src/prelude/unsupported.ts). */
const UNSUPPORTED = new Set([
  'addInitScript',
  'addLocatorHandler',
  'addScriptTag',
  'addStyleTag',
  'bringToFront',
  'cancelPickLocator',
  'consoleMessages',
  'drop',
  'emulateMedia',
  'evaluateHandle',
  'exposeBinding',
  'exposeFunction',
  'frameElement',
  'getProperties',
  'getProperty',
  'hideHighlight',
  'highlight',
  'ownerFrame',
  'pageErrors',
  'pause',
  'pickLocator',
  'removeLocatorHandler',
  'requestGC',
  'route',
  'routeFromHAR',
  'routeWebSocket',
  'setExtraHTTPHeaders',
  'setViewportSize',
  'tap',
  'unroute',
  'unrouteAll',
  'waitForNavigation',
  'waitForRequest',
  'waitForResponse',
])
/** Members Neo refuses on one receiver only. */
const UNSUPPORTED_ON = {
  handle: new Set(['contentFrame', 'jsonValue', 'waitForSelector']),
  locator: new Set(['normalize', 'waitForFunction']),
}
const UNSUPPORTED_PROPERTIES = new Set([
  'accessibility',
  'clock',
  'coverage',
  'request',
  'screencast',
  'touchscreen',
  'tracing',
])
/** Page ops Neo serves only in the main frame so far (page/world_call.rs main_frame_only). */
const MAIN_FRAME_ONLY = new Set(['content', 'title', 'waitForFunction'])

/** What a member call means to Neo: null when it is built, or why the case cannot run yet. */
function memberStatus(name, kind) {
  if (UNSUPPORTED.has(name) || UNSUPPORTED_ON[kind]?.has(name))
    return { unsupported: `${kind === 'unknown' ? '' : `${kind}.`}${name}` }
  if (name === 'evaluate' && kind === 'frame')
    return { needs: 'unowned', what: 'evaluate in a child frame' }
  if (kind === 'frame' && MAIN_FRAME_ONLY.has(name))
    return { needs: 'unowned', what: `${name} in a child frame` }
  if (name === 'screenshot' && (kind === 'locator' || kind === 'handle'))
    return { needs: 'unowned', what: 'element screenshots' }
  return null
}

const WEB_FIRST = new Set([
  'toBeAttached',
  'toBeChecked',
  'toBeDisabled',
  'toBeEditable',
  'toBeEmpty',
  'toBeEnabled',
  'toBeFocused',
  'toBeHidden',
  'toBeInViewport',
  'toBeVisible',
  'toContainClass',
  'toContainText',
  'toHaveAccessibleDescription',
  'toHaveAccessibleErrorMessage',
  'toHaveAccessibleName',
  'toHaveAttribute',
  'toHaveClass',
  'toHaveCount',
  'toHaveCSS',
  'toHaveId',
  'toHaveRole',
  'toHaveText',
  'toHaveTitle',
  'toHaveURL',
  'toHaveValue',
  'toHaveValues',
  'toPass',
])
/** Generic matchers, including pageTest.ts's own `toContainYaml` (a string containment). */
const GENERIC = new Set([
  'toBe',
  'toBeCloseTo',
  'toBeDefined',
  'toBeFalsy',
  'toBeGreaterThan',
  'toBeGreaterThanOrEqual',
  'toBeInstanceOf',
  'toBeLessThan',
  'toBeLessThanOrEqual',
  'toBeNaN',
  'toBeNull',
  'toBeTruthy',
  'toBeUndefined',
  'toContain',
  'toContainEqual',
  'toContainYaml',
  'toEqual',
  'toHaveLength',
  'toHaveProperty',
  'toMatch',
  'toMatchObject',
  'toStrictEqual',
  'toThrow',
  'toThrowError',
])
/** The ordering matchers, whose verdict a lifted case records (see the header). */
const ORDERING = {
  toBeGreaterThan: '>',
  toBeGreaterThanOrEqual: '>=',
  toBeLessThan: '<',
  toBeLessThanOrEqual: '<=',
}
/**
 * Matchers Neo refuses: unsupported.ts's expect entries, and toHaveJSProperty, which the crate
 * refuses because the sealed world never sees the page's own objects (expect/expectation.rs).
 */
const UNSUPPORTED_MATCHERS = new Set([
  'toBeOK',
  'toHaveJSProperty',
  'toHaveScreenshot',
  'toMatchAriaSnapshot',
  'toMatchSnapshot',
])
/** Node's globals a script in Neo's run does not have; `Buffer` it has (prelude/buffer.ts). */
const NODE_ONLY = new Set([
  '__dirname',
  '__filename',
  'exports',
  'global',
  'module',
  'process',
  'require',
])
/**
 * Globals only the 1.62 `signal` options use, which Neo v1 leaves out (design.md); claw-sdk's
 * runtime has no AbortController, so such a test fails before its first Playwright call.
 */
const SIGNAL_ONLY = new Set(['AbortController', 'AbortSignal'])
const LOCATOR_MAKERS = new Set([
  'and',
  'describe',
  'filter',
  'first',
  'getByAltText',
  'getByLabel',
  'getByPlaceholder',
  'getByRef',
  'getByRole',
  'getByTestId',
  'getByText',
  'getByTitle',
  'last',
  'locator',
  'nth',
  'or',
  'owner',
  'visible',
])
/** Upstream reasons a test is not lifted; skipped.json groups by these. */
const SKIP_KINDS = {
  asset: 'loads an upstream asset that is not vendored',
  fixture: 'uses a fixture the runners do not provide',
  internal:
    "uses Playwright's internal test hooks, which only its own test mode honors",
  node: 'uses Node itself',
  server: "uses upstream's test server beyond its static assets",
  shape: 'is written in a way the converter does not lift',
  unsupported: 'uses a member Neo does not support',
  'upstream-skip': 'upstream does not run it on Chromium',
}

// ---- inputs ----------------------------------------------------------------------------

const check = process.argv.includes('--check')
const nodeModules = process.env.PLAYWRIGHT_NODE_MODULES
if (!nodeModules) {
  console.error('convert: set PLAYWRIGHT_NODE_MODULES')
  process.exit(2)
}
const [tag] = readFileSync(resolve(upstream, 'VERSION'), 'utf8').split('\n')
const playwrightVersion = JSON.parse(
  readFileSync(resolve(nodeModules, 'playwright/package.json'), 'utf8'),
).version
if (`v${playwrightVersion}` !== tag) {
  console.error(
    `convert: playwright is ${playwrightVersion}; the vendored copy is ${tag}`,
  )
  process.exit(2)
}
const { babelParse, traverse, types } = await import(
  resolve(nodeModules, 'playwright/lib/transform/babelBundle.js')
).then((module) => module.default ?? module)

const manifest = new Map(
  readFileSync(resolve(upstream, 'SHA256SUMS'), 'utf8')
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      const [hash, path] = line.split('  ')
      return [path, hash]
    }),
)

/** A pinned upstream file's text, refused when it is not the bytes sync.sh recorded. */
function pinned(path) {
  const bytes = readFileSync(resolve(upstream, path))
  const hash = createHash('sha256').update(bytes).digest('hex')
  if (manifest.get(path) !== hash)
    throw new Error(`${path} is not the file SHA256SUMS pins; run sync.sh`)
  return bytes.toString('utf8')
}

const specs = [...manifest.keys()]
  .filter((path) => path.startsWith('tests/page/') && path.endsWith('.spec.ts'))
  .sort()
for (const spec of specs)
  if (!SPECS.some((pattern) => pattern.test(basename(spec))))
    throw new Error(`${spec} is vendored but no SPECS pattern selects it`)
const assets = new Set(
  [...manifest.keys()].filter((path) => path.startsWith('tests/assets/')),
)

// ---- one parsed source file and its edits ----------------------------------------------

/**
 * A parsed upstream file and the edits that turn any range of it into a case's JavaScript.
 * Edits are AST nodes, so two edits are disjoint or nested; `text` applies the outermost ones
 * in a range, and each edit's render applies the ones nested in it the same way.
 */
class Source {
  constructor(path) {
    this.path = path
    this.code = pinned(path)
    this.ast = babelParse(this.code, path, true)
    this.edits = []
    traverse(this.ast, {
      Program: (program) => {
        this.program = program
      },
      enter: (path) => this.collect(path),
    })
    this.edits.sort((a, b) => a.start - b.start || b.end - a.end)
    for (let index = 1; index < this.edits.length; index++) {
      const [before, edit] = [this.edits[index - 1], this.edits[index]]
      if (before.start === edit.start && before.end === edit.end)
        throw new Error(
          `${path}:${this.line(edit.start)}: two edits on one node`,
        )
    }
  }

  line(offset) {
    return this.code.slice(0, offset).split('\n').length
  }

  /** The case text of [start, end), every edit in it applied; `self` is the calling edit. */
  text(start, end, self = null) {
    let out = ''
    let at = start
    for (const edit of this.edits) {
      if (edit === self || edit.start < at || edit.end > end) continue
      out += this.code.slice(at, edit.start) + edit.render(edit)
      at = edit.end
    }
    return out + this.code.slice(at, end)
  }

  nodeText(node) {
    return this.text(node.start, node.end)
  }

  edit(start, end, render) {
    this.edits.push({ start, end, render })
  }

  /** A statement removed together with its own line when it has the line to itself. */
  remove(node) {
    let { start, end } = node
    const lineStart = this.code.lastIndexOf('\n', start - 1) + 1
    const rest = /^[ \t]*\n/.exec(this.code.slice(end))
    if (/^[ \t]*$/.test(this.code.slice(lineStart, start)) && rest) {
      start = lineStart
      end += rest[0].length
    }
    this.edit(start, end, () => '')
  }

  /** The edit, if any, for one node: the TypeScript strip and the four lift rewrites. */
  collect(path) {
    const { node } = path
    switch (node.type) {
      case 'TSTypeAnnotation':
      case 'TSTypeParameterInstantiation':
      case 'TSTypeParameterDeclaration':
        this.edit(node.start, node.end, () => '')
        return
      case 'TSTypeAliasDeclaration':
      case 'TSInterfaceDeclaration':
      case 'TSDeclareFunction':
        this.remove(node)
        return
      case 'TSAsExpression':
      case 'TSSatisfiesExpression':
        this.edit(node.start, node.end, (self) =>
          this.text(node.start, node.typeAnnotation.start, self).replace(
            /\s*\b(?:as|satisfies)\s*$/,
            '',
          ),
        )
        return
      case 'TSNonNullExpression':
        if (this.code[node.end - 1] !== '!')
          throw new Error(
            `${this.path}:${this.line(node.start)}: a non-null assertion`,
          )
        this.edit(node.start, node.end, (self) =>
          this.text(node.start, node.end - 1, self),
        )
        return
      case 'Identifier':
        if (node.optional) this.edit(node.start, node.end, () => node.name)
        return
      case 'MemberExpression':
        if (
          !node.computed &&
          node.object.type === 'Identifier' &&
          node.object.name === 'window' &&
          node.property.name === 'builtins'
        )
          this.edit(node.start, node.end, () => 'window')
        else if (
          !node.computed &&
          node.property.name === 'soft' &&
          pageTestImport(path, node.object) === 'expect'
        )
          this.edit(node.start, node.end, () => 'expect')
        return
      case 'ExpressionStatement':
        if (removedStatement(path)) this.remove(node)
        return
      case 'CallExpression':
        return this.collectCall(path)
    }
  }

  collectCall(path) {
    const { node } = path
    const chain = expectChain(path)
    if (chain !== null) {
      const key = `'L${chain.line}'`
      if (chain.kind === 'web') {
        // As a statement, a failed assertion is recorded and the test goes on. Used as a
        // value (`.catch(e => e)`, Promise.all), it must still reject as upstream's does.
        const parent = path.parentPath
        const statement =
          parent.isExpressionStatement() ||
          (parent.isAwaitExpression() &&
            parent.parentPath.isExpressionStatement())
        const method = statement ? 'record' : 'pass'
        this.edit(node.start, node.end, (self) => {
          return `tier2.${method}(${key}, () => ${this.text(node.start, node.end, self)})`
        })
        return
      }
      if (chain.received === undefined) return
      const awaits = hasDirectAwait(chain.received)
      const calls =
        (chain.matcher === 'toThrow' || chain.matcher === 'toThrowError') &&
        !chain.modifiers.some(
          (name) => name === 'resolves' || name === 'rejects',
        )
      // toMatch states the values upstream accepts as a pattern (one test's own comment says a
      // race "may re-number the main frame, so accept any ref"), so it records the verdict.
      const [pattern] = node.arguments
      const matches =
        chain.matcher === 'toMatch' &&
        ['RegExpLiteral', 'StringLiteral'].includes(pattern?.type)
      // A bound is what upstream accepts too: a count of scroll events varies from run to run
      // in both runners, and only whether it stays under the bound is upstream's to judge.
      const order =
        node.arguments.length === 1 ? ORDERING[chain.matcher] : undefined
      this.edit(node.start, node.end, () => {
        let received = this.nodeText(chain.received)
        if (calls) received = `(${received})()`
        if (matches)
          received = `tier2.matches(${received}, ${this.nodeText(pattern)})`
        if (order !== undefined)
          received = `(${received}) ${order} (${this.nodeText(pattern)})`
        const thunk = `${awaits ? 'async ' : ''}() => (${received})`
        const call = `tier2.record(${key}, ${thunk})`
        return awaits ? `(await ${call})` : call
      })
      return
    }
    const callee = node.callee
    if (callee.type !== 'MemberExpression' || callee.computed) return
    const name = callee.property.name
    if (name === 'setContent' && node.arguments.length > 0) {
      const [html, ...rest] = node.arguments
      this.edit(node.start, node.end, () => {
        const options =
          rest.length === 0
            ? ''
            : `, ${this.text(rest[0].start, rest.at(-1).end)}`
        return `${this.text(node.start, callee.property.start)}goto(tier2.html(${this.nodeText(html)})${options})`
      })
    } else if (name === 'step' && isItRef(path, callee.object)) {
      const body = node.arguments.at(-1)
      this.edit(node.start, node.end, () => `(${this.nodeText(body)})()`)
    }
  }
}

// ---- recognizing upstream's test API ---------------------------------------------------

/** The name `node` imports from pageTest.ts, if it is an Identifier bound to that import. */
function pageTestImport(path, node) {
  if (node?.type !== 'Identifier') return null
  const binding = path.scope.getBinding(node.name)
  if (binding?.kind !== 'module') return null
  const declaration = binding.path.parentPath.node
  if (declaration.source.value !== './pageTest') return null
  return binding.path.node.imported?.name ?? null
}
const isItRef = (path, node) => pageTestImport(path, node) === 'test'

/** `it.a.b(...)`'s member names, ['a', 'b'], or null when the callee is not `it`'s. */
function itChain(path, callee) {
  const names = []
  let node = callee
  while (node.type === 'MemberExpression' && !node.computed) {
    names.unshift(node.property.name)
    node = node.object
  }
  return isItRef(path, node) ? names : null
}

/** The names pageTest.ts re-exports from utils.ts (`export { rafraf } from '../config/utils'`). */
const reexported = new Set(
  [
    ...pinned(PAGE_TEST).matchAll(
      /^export \{([^}]+)\} from '\.\.\/config\/utils';$/gm,
    ),
  ]
    .flatMap((match) => match[1].split(','))
    .map((name) => name.trim()),
)

/** In-body `it.skip(..)` and the other modifiers, and `it.info().annotations.push(..)`. */
function removedStatement(path) {
  const expression = path.node.expression
  if (expression.type !== 'CallExpression') return false
  if (!path.getFunctionParent()) return false
  const names = itChain(path, expression.callee)
  if (names?.length === 1 && MODIFIERS.has(names[0])) return true
  return isAnnotationPush(path, expression)
}
const MODIFIERS = new Set(['skip', 'fixme', 'fail', 'slow'])

function isAnnotationPush(path, call) {
  const push = call.callee
  if (push.type !== 'MemberExpression' || push.property.name !== 'push')
    return false
  const annotations = push.object
  if (
    annotations.type !== 'MemberExpression' ||
    annotations.property.name !== 'annotations'
  )
    return false
  const info = annotations.object
  if (info.type !== 'CallExpression') return false
  const names = itChain(path, info.callee)
  return names?.length === 1 && names[0] === 'info'
}

/**
 * `expect(v).not.toBe(x)` and its kin: the matcher, its modifiers, the received node, the
 * line, and whether it is a web-first (or polling) assertion or a generic one.
 */
function expectChain(path) {
  const { node } = path
  const callee = node.callee
  if (callee?.type !== 'MemberExpression' || callee.computed) return null
  const matcher = callee.property.name
  const modifiers = []
  let object = callee.object
  while (
    object.type === 'MemberExpression' &&
    !object.computed &&
    ['not', 'resolves', 'rejects'].includes(object.property.name)
  ) {
    modifiers.push(object.property.name)
    object = object.object
  }
  if (object.type !== 'CallExpression') return null
  let flavor = null
  if (pageTestImport(path, object.callee) === 'expect') flavor = 'expect'
  else if (
    object.callee.type === 'MemberExpression' &&
    !object.callee.computed &&
    pageTestImport(path, object.callee.object) === 'expect' &&
    ['soft', 'poll'].includes(object.callee.property.name)
  )
    flavor = object.callee.property.name
  if (flavor === null) return null
  let kind = 'unknown'
  if (flavor === 'poll' || WEB_FIRST.has(matcher)) kind = 'web'
  else if (GENERIC.has(matcher)) kind = 'generic'
  else if (UNSUPPORTED_MATCHERS.has(matcher)) kind = 'unsupported'
  return {
    base: object,
    flavor,
    kind,
    line: object.loc.start.line,
    matcher,
    modifiers,
    received: object.arguments[0],
  }
}

function hasDirectAwait(node) {
  if (node === null || typeof node !== 'object') return false
  if (node.type === 'AwaitExpression') return true
  if (types.isFunction(node)) return false
  for (const key of types.VISITOR_KEYS[node.type] ?? []) {
    const child = node[key]
    if (
      Array.isArray(child) ? child.some(hasDirectAwait) : hasDirectAwait(child)
    )
      return true
  }
  return false
}

// ---- static values: skip conditions, titles and loops ----------------------------------

const UNKNOWN = Symbol('unknown')

/** The value of a constant expression, or UNKNOWN; && and || decide what they can. */
function constant(node, env) {
  const evaluate = CONSTANT[node?.type]
  return evaluate === undefined ? UNKNOWN : evaluate(node, env)
}

const literal = (node) => node.value
const COMPARE = {
  '===': (left, right) => left === right,
  '!==': (left, right) => left !== right,
}
const STRING_TESTS = new Set(['includes', 'startsWith', 'endsWith'])

/** One evaluator per node type an upstream skip condition, title or loop is written with. */
const CONSTANT = {
  StringLiteral: literal,
  NumericLiteral: literal,
  BooleanLiteral: literal,
  NullLiteral: () => null,
  Identifier: (node, env) => {
    if (node.name === 'undefined') return undefined
    return node.name in env ? env[node.name] : UNKNOWN
  },
  TemplateLiteral: (node, env) => {
    const parts = node.expressions.map((expression) =>
      constant(expression, env),
    )
    if (parts.includes(UNKNOWN)) return UNKNOWN
    return node.quasis
      .map(
        (quasi, index) =>
          quasi.value.cooked +
          (index < parts.length ? String(parts[index]) : ''),
      )
      .join('')
  },
  UnaryExpression: (node, env) => {
    const value = constant(node.argument, env)
    return node.operator !== '!' || value === UNKNOWN ? UNKNOWN : !value
  },
  LogicalExpression: (node, env) => {
    const left = constant(node.left, env)
    const right = constant(node.right, env)
    if (node.operator === '??')
      return left === UNKNOWN ? UNKNOWN : (left ?? right)
    const and = node.operator === '&&'
    // A known left decides; an unknown left still gives a known answer when the right
    // alone settles it (`unknown && false`, `unknown || true`).
    if (left !== UNKNOWN) return Boolean(left) === and ? right : left
    return right !== UNKNOWN && Boolean(right) !== and ? !and : UNKNOWN
  },
  BinaryExpression: (node, env) => {
    const left = constant(node.left, env)
    const right = constant(node.right, env)
    const compare = COMPARE[node.operator]
    if (left === UNKNOWN || right === UNKNOWN || !compare) return UNKNOWN
    return compare(left, right)
  },
  ConditionalExpression: (node, env) => {
    const test = constant(node.test, env)
    if (test === UNKNOWN) return UNKNOWN
    return constant(test ? node.consequent : node.alternate, env)
  },
  ArrayExpression: (node, env) => {
    const values = node.elements.map((element) => constant(element, env))
    return values.includes(UNKNOWN) ? UNKNOWN : values
  },
  ObjectExpression: (node, env) => {
    const plain = node.properties.every(
      (property) => property.type === 'ObjectProperty' && !property.computed,
    )
    if (!plain) return UNKNOWN
    const entries = node.properties.map((property) => [
      property.key.name ?? property.key.value,
      constant(property.value, env),
    ])
    return entries.some(([, value]) => value === UNKNOWN)
      ? UNKNOWN
      : Object.fromEntries(entries)
  },
  // A fixture condition, `({ browserName }) => browserName === 'webkit'`.
  ArrowFunctionExpression: (node, env) =>
    node.body.type === 'BlockStatement' ? UNKNOWN : constant(node.body, env),
  CallExpression: (node, env) => {
    const callee = node.callee
    if (callee.type !== 'MemberExpression' || callee.computed) return UNKNOWN
    const args = node.arguments.map((arg) => constant(arg, env))
    if (args.includes(UNKNOWN)) return UNKNOWN
    const method = callee.property.name
    if (callee.object.type === 'Identifier' && callee.object.name === 'Object')
      return method === 'entries' ? Object.entries(args[0]) : UNKNOWN
    const target = constant(callee.object, env)
    if (target === UNKNOWN || target == null || !STRING_TESTS.has(method))
      return UNKNOWN
    return target[method](...args)
  },
}

/** The names a loop's `const <pattern>` binds to one value of its iterable. */
function bind(pattern, value) {
  if (pattern.type === 'Identifier') return { [pattern.name]: value }
  if (pattern.type === 'ArrayPattern' && Array.isArray(value)) {
    const bound = {}
    pattern.elements.forEach((element, index) => {
      if (element?.type === 'Identifier') bound[element.name] = value[index]
    })
    return bound
  }
  return null
}

// ---- collecting tests ------------------------------------------------------------------

/**
 * One upstream test as a lift sees it: where it is, its title path, its function, and what its
 * describe scopes add (an upstream skip, notes, loop bindings).
 * @typedef {{ source: Source, line: number, titles: string[], fn: object,
 *   scope: Scope }} UpstreamTest
 * @typedef {{ titles: string[], skip: Skip | null, notes: string[], env: object,
 *   bindings: string[] }} Scope
 * @typedef {{ kind: keyof SKIP_KINDS, detail: string }} Skip
 */

function scopeSkip(scope, kind, detail) {
  return { ...scope, skip: scope.skip ?? { kind, detail } }
}

/** A modifier's verdict: { skip } for skip and fixme, { note } for fail, or null. */
function modifierVerdict(method, args, env, source) {
  if (method === 'slow') return null
  const [condition, reason] = args
  const why = reason?.type === 'StringLiteral' ? `: ${reason.value}` : ''
  const value =
    condition === undefined
      ? true
      : constant(condition, { ...FIXTURE_VALUES, ...env })
  if (value === UNKNOWN) {
    const text = source.code.slice(condition.start, condition.end)
    return method === 'fail'
      ? { note: `upstream may expect it to fail here (it.fail(${text}))${why}` }
      : {
          skip: {
            kind: 'upstream-skip',
            detail: `it.${method}(${text}) depends on what the converter cannot know${why}`,
          },
        }
  }
  if (!value) return null
  return method === 'fail'
    ? { note: `upstream expects it to fail on Chromium${why}` }
    : { skip: { kind: 'upstream-skip', detail: `it.${method}${why}` } }
}

function isTitleFirst(args) {
  return (
    args.length >= 2 &&
    ['StringLiteral', 'TemplateLiteral'].includes(args[0].type) &&
    types.isFunction(args.at(-1))
  )
}

/** Every test in `statements` (one scope's body), in source order. */
function collectScope(source, statements, scope, tests) {
  const calls = statements.map((statement) => {
    if (!statement.isExpressionStatement()) return null
    const expression = statement.node.expression
    if (expression.type !== 'CallExpression') return null
    const names = itChain(statement, expression.callee)
    return names === null ? null : { names, call: statement.get('expression') }
  })
  // A scope's modifiers and options cover every test in it, wherever they stand.
  for (const entry of calls) {
    if (entry === null) continue
    const { names, call } = entry
    const args = call.node.arguments
    if (names.length === 1 && MODIFIERS.has(names[0]) && !isTitleFirst(args)) {
      const verdict = modifierVerdict(names[0], args, scope.env, source)
      if (verdict?.skip) scope = { ...scope, skip: scope.skip ?? verdict.skip }
      if (verdict?.note)
        scope = { ...scope, notes: [...scope.notes, verdict.note] }
    } else if (names.length === 1 && names[0] === 'use') {
      scope = scopeSkip(scope, 'fixture', 'test.use(...) options')
    } else if (
      names.length === 1 &&
      ['beforeEach', 'afterEach', 'beforeAll', 'afterAll'].includes(names[0])
    ) {
      scope = scopeSkip(scope, 'shape', `a ${names[0]} hook`)
    }
  }
  statements.forEach((statement, index) => {
    if (isDeclaration(statement)) return
    if (statement.isForOfStatement()) {
      collectLoop(source, statement, scope, tests)
      return
    }
    if (calls[index] === null)
      throw new Error(
        `${source.path}:${statement.node.loc.start.line}: a statement the converter does not know`,
      )
    collectCall(source, calls[index], scope, tests)
  })
}

/** Declarations a scope may hold besides tests; a test reaches them as helpers. */
function isDeclaration(statement) {
  const { node } = statement
  return (
    statement.isImportDeclaration() ||
    statement.isFunctionDeclaration() ||
    statement.isVariableDeclaration() ||
    statement.isClassDeclaration() ||
    node.type.startsWith('TS') ||
    (statement.isExportNamedDeclaration() && node.declaration !== null)
  )
}

/** One `it...(...)` statement of a scope: a test, a titled modifier, or a describe. */
function collectCall(source, { names, call }, scope, tests) {
  const args = call.node.arguments
  const [first] = names
  if (names.length === 0 || (names.length === 1 && first === 'only')) {
    collectTest(source, call, scope, null, tests)
  } else if (names.length === 1 && MODIFIERS.has(first) && isTitleFirst(args)) {
    collectTest(source, call, scope, TITLED_VERDICTS[first], tests)
  } else if (first === 'describe' && names[1] !== 'configure') {
    const fn = call.get(`arguments.${args.length - 1}`)
    const title = args.length > 1 ? constant(args[0], scope.env) : ''
    let child = {
      ...scope,
      titles: title === '' ? scope.titles : [...scope.titles, String(title)],
    }
    if (names.includes('skip') || names.includes('fixme'))
      child = scopeSkip(child, 'upstream-skip', `it.${names.join('.')}`)
    collectScope(source, fn.get('body.body'), child, tests)
  }
}

/** `it.skip('title', fn)` and its kin, which mark one test. */
const TITLED_VERDICTS = {
  fail: { note: 'upstream expects it to fail' },
  fixme: { skip: { kind: 'upstream-skip', detail: 'it.fixme(title, ...)' } },
  skip: { skip: { kind: 'upstream-skip', detail: 'it.skip(title, ...)' } },
  slow: null,
}

/** A `for (const x of [...])` that defines tests: one scope per value, when it is constant. */
function collectLoop(source, statement, scope, tests) {
  const { left, right, body } = statement.node
  const declarator = left.declarations?.[0]
  const values = constant(right, { ...FIXTURE_VALUES, ...scope.env })
  const bodyPaths =
    body.type === 'BlockStatement'
      ? statement.get('body.body')
      : [statement.get('body')]
  if (left.kind !== 'const' || !Array.isArray(values) || !declarator) {
    const stuck = scopeSkip(
      scope,
      'shape',
      'defined in a loop over values the converter cannot compute',
    )
    collectScope(source, bodyPaths, stuck, tests)
    return
  }
  const pattern = source.code.slice(declarator.id.start, declarator.id.end)
  for (const value of values) {
    const bound = bind(declarator.id, value)
    if (bound === null)
      throw new Error(`${source.path}:${left.loc.start.line}: a loop pattern`)
    collectScope(
      source,
      bodyPaths,
      {
        ...scope,
        env: { ...scope.env, ...bound },
        bindings: [
          ...scope.bindings,
          `const ${pattern} = ${JSON.stringify(value)}`,
        ],
      },
      tests,
    )
  }
}

function collectTest(source, call, scope, verdict, tests) {
  const args = call.node.arguments
  const title = constant(args[0], scope.env)
  const fn = call.get(`arguments.${args.length - 1}`)
  let testScope = scope
  if (verdict?.skip)
    testScope = { ...testScope, skip: testScope.skip ?? verdict.skip }
  if (verdict?.note)
    testScope = { ...testScope, notes: [...testScope.notes, verdict.note] }
  if (title === UNKNOWN)
    testScope = scopeSkip(
      testScope,
      'shape',
      'a title the converter cannot compute',
    )
  tests.push({
    source,
    line: call.node.loc.start.line,
    titles: [
      ...scope.titles,
      title === UNKNOWN ? source.code.slice(args[0].start, args[0].end) : title,
    ],
    fn,
    scope: testScope,
  })
}

// ---- analyzing one test ----------------------------------------------------------------

/**
 * What one test needs, found by walking its runner-side code (functions that run in the page
 * are not walked) and every helper it calls. The first Skip wins.
 */
class Lift {
  constructor(test) {
    this.test = test
    this.skip = test.scope.skip
    this.notes = [...test.scope.notes]
    this.needs = new Map()
    this.constants = new Set()
    /** The CASE_FIXTURES the test uses. */
    this.fixtures = new Set()
    /** Inlined declarations by `${path}:${start}`: { source, node }. */
    this.helpers = new Map()
  }

  refuse(kind, detail) {
    if (!(kind in SKIP_KINDS)) throw new Error(`no skip kind ${kind}`)
    this.skip ??= { kind, detail }
  }

  need(unit, what) {
    if (!(unit in UNITS)) throw new Error(`no unit ${unit}`)
    if (!this.needs.has(unit)) this.needs.set(unit, new Set())
    this.needs.get(unit).add(what)
  }
}

/** The kind of object an expression is, from how it was made: page, locator, handle... */
function kindOf(path, depth = 0) {
  if (depth > 8 || !path?.node) return 'unknown'
  const infer = KIND_OF[path.node.type]
  return infer === undefined ? 'unknown' : infer(path, depth + 1)
}

/** What a call returns, by the member it calls and the kind it calls it on. */
function kindOfCall(name, base) {
  if (LOCATOR_MAKERS.has(name))
    return base === 'frameLocator' && ['first', 'last', 'nth'].includes(name)
      ? 'frameLocator'
      : 'locator'
  if (name === 'contentFrame')
    return base === 'handle' ? 'frame' : 'frameLocator'
  if (name === 'all') return base === 'locator' ? 'locators' : 'unknown'
  return CALL_KINDS[name] ?? 'unknown'
}
const CALL_KINDS = {
  $: 'handle',
  $$: 'handles',
  childFrames: 'frames',
  elementHandle: 'handle',
  elementHandles: 'handles',
  frame: 'frame',
  frameLocator: 'frameLocator',
  frames: 'frames',
  mainFrame: 'page',
  newPage: 'page',
  waitForSelector: 'handle',
}

function kindOfIdentifier(path, depth) {
  const { name } = path.node
  if (/^(page\d*|newPage|popup|otherPage)$/.test(name)) return 'page'
  const binding = path.scope.getBinding(name)
  if (binding?.kind === 'param') return kindOfParam(binding.identifier, name)
  if (binding?.path.isVariableDeclarator()) {
    if (binding.path.node.init) return kindOf(binding.path.get('init'), depth)
    const loop = binding.path.parentPath.parentPath
    if (loop.isForOfStatement()) return single(kindOf(loop.get('right'), depth))
  }
  return /frame/i.test(name) ? 'frame' : 'unknown'
}

function kindOfMember(path, depth) {
  const { node } = path
  if (node.computed) return single(kindOf(path.get('object'), depth))
  const name = node.property.name
  return name === 'keyboard' || name === 'mouse' ? name : 'unknown'
}

function kindOfCallPath(path, depth) {
  const callee = path.node.callee
  if (!callee.property || callee.computed) return 'unknown'
  return kindOfCall(
    callee.property.name,
    kindOf(path.get('callee.object'), depth),
  )
}

const KIND_OF = {
  Identifier: kindOfIdentifier,
  AwaitExpression: (path, depth) => kindOf(path.get('argument'), depth),
  TSNonNullExpression: (path, depth) => kindOf(path.get('expression'), depth),
  TSAsExpression: (path, depth) => kindOf(path.get('expression'), depth),
  MemberExpression: kindOfMember,
  OptionalMemberExpression: kindOfMember,
  ArrayExpression: (path, depth) => {
    const kinds = new Set(
      path.get('elements').map((element) => kindOf(element, depth)),
    )
    return kinds.size === 1 ? `${[...kinds][0]}s` : 'unknown'
  },
  CallExpression: kindOfCallPath,
  OptionalCallExpression: kindOfCallPath,
}

function single(kind) {
  return kind.endsWith('s') ? kind.slice(0, -1) : 'unknown'
}

/** A parameter's kind from its TypeScript annotation (`target: Page | Frame`) or its name. */
function kindOfParam(param, name) {
  const annotation = param.typeAnnotation?.typeAnnotation
  const first =
    annotation?.type === 'TSUnionType' ? annotation.types[0] : annotation
  const typeName = first?.typeName?.name
  const byType = {
    ElementHandle: 'handle',
    Frame: 'frame',
    Locator: 'locator',
    Page: 'page',
  }[typeName]
  if (byType) return byType
  if (/^page\d*$/.test(name)) return 'page'
  return /frame/i.test(name) ? 'frame' : 'unknown'
}

/** Walks `root` (a test function or an inlined helper) for what the lift needs. */
function scan(lift, root, { test = false } = {}) {
  const skipped = new Set()
  const pageFunctions = new Set()
  const webRanges = []
  root.traverse({
    enter(path) {
      const { node } = path
      if (
        skipped.has(node) ||
        (node.type.startsWith('TS') &&
          ![
            'TSAsExpression',
            'TSSatisfiesExpression',
            'TSNonNullExpression',
          ].includes(node.type))
      ) {
        path.skip()
        return
      }
      if (path.isCallExpression() || path.isOptionalCallExpression()) call(path)
      else if (
        path.isObjectProperty() &&
        /^__testHook/.test(node.key.name ?? node.key.value)
      )
        lift.refuse(
          'internal',
          `passes Playwright's ${node.key.name ?? node.key.value} option`,
        )
      else if (path.isMemberExpression() || path.isOptionalMemberExpression())
        member(path)
      else if (path.isIdentifier() && path.isReferencedIdentifier())
        reference(path)
    },
  })

  function call(path) {
    const { node } = path
    const names = itChain(path, node.callee)
    if (names !== null) return itCall(path, names)
    const chain = expectChain(path)
    if (chain !== null) return assertion(path, chain)
    const callee = node.callee
    if (callee.type === 'Identifier' && fixtureOf(path, callee) === 'asset')
      return assetFile(node.arguments[0])
    if (!callee.property || callee.computed) return
    const name = callee.property.name
    const pageArg = PAGE_FUNCTION_ARG.get(name)
    if (pageArg !== undefined) {
      const arg = node.arguments[pageArg]
      if (types.isFunction(arg)) skipped.add(arg)
      else if (arg?.type === 'Identifier') pageFunctions.add(arg)
    }
    const status = memberStatus(name, kindOf(path.get('callee.object')))
    if (status?.needs) lift.need(status.needs, status.what)
    if (status?.unsupported)
      lift.refuse(
        'unsupported',
        `${status.unsupported} is not supported in Neo`,
      )
  }

  function itCall(path, names) {
    const [first] = names
    const statement = path.parentPath
    if (
      names.length === 1 &&
      MODIFIERS.has(first) &&
      statement.isExpressionStatement()
    ) {
      const verdict = modifierVerdict(
        first,
        path.node.arguments,
        lift.test.scope.env,
        lift.test.source,
      )
      if (verdict?.skip) lift.refuse(verdict.skip.kind, verdict.skip.detail)
      if (verdict?.note) lift.notes.push(verdict.note)
      for (const arg of path.node.arguments) skipped.add(arg)
      return
    }
    if (names.length === 1 && first === 'info') {
      const push = path.parentPath.parentPath.parentPath
      if (
        push.isCallExpression() &&
        isAnnotationPush(push, push.node) &&
        push.parentPath.isExpressionStatement()
      ) {
        for (const arg of push.node.arguments) skipped.add(arg)
        return
      }
      return lift.refuse('shape', 'reads its test info')
    }
    if (names.length === 1 && first === 'step') return
    lift.refuse('shape', `calls it.${names.join('.')} inside the test`)
  }

  function assertion(path, chain) {
    if (chain.received === undefined)
      return lift.refuse('shape', 'an expect() with no value')
    if (chain.kind === 'unknown')
      return lift.refuse('shape', `the ${chain.matcher} matcher`)
    if (chain.kind === 'unsupported')
      return lift.refuse(
        'unsupported',
        `expect(...).${chain.matcher} is not supported in Neo`,
      )
    if (chain.kind === 'web') {
      webRanges.push([path.node.start, path.node.end])
      return
    }
    for (const arg of path.node.arguments) skipped.add(arg)
    for (const arg of chain.base.arguments.slice(1)) skipped.add(arg)
    skipped.add(chain.base.callee)
  }

  function member(path) {
    const { node } = path
    if (node.computed) return
    const name = node.property.name
    const object = node.object
    if (
      object.type === 'Identifier' &&
      object.name === 'server' &&
      fixtureOf(path, object) === 'server'
    ) {
      if (!SERVER_MEMBERS.has(name))
        return lift.refuse('server', `server.${name}`)
      lift.fixtures.add('server')
      asset(path, name)
      return
    }
    if (UNSUPPORTED_PROPERTIES.has(name)) {
      const kind = kindOf(path.get('object'))
      if (
        kind === 'page' ||
        (object.type === 'Identifier' && object.name === 'context')
      )
        lift.refuse(
          'unsupported',
          `${kind === 'page' ? 'page' : 'context'}.${name} is not supported in Neo`,
        )
    }
  }

  /** The asset a `server.PREFIX + '/x.html'` loads must be vendored. */
  function asset(path, name) {
    if (name === 'EMPTY_PAGE') return
    const parent = path.parentPath.node
    let file = null
    if (
      parent.type === 'BinaryExpression' &&
      parent.left === path.node &&
      parent.right.type === 'StringLiteral'
    )
      file = parent.right.value
    else if (parent.type === 'TemplateLiteral') {
      const index = parent.expressions.indexOf(path.node)
      file = parent.quasis[index + 1].value.cooked
    }
    // The path ends where the URL does inside markup: `src="${server.PREFIX}/x.html"></iframe>`.
    file = /^\/[^\s"'`<>()?#]*/.exec(file ?? '')?.[0] ?? null
    if (file === null) return
    const vendored = `tests/assets${file}`
    if (vendored !== 'tests/assets/' && !assets.has(vendored))
      lift.refuse('asset', `${file} (add it to sync.sh)`)
  }

  /** The file `asset('x.txt')` names must be vendored. */
  function assetFile(arg) {
    if (arg?.type !== 'StringLiteral') return
    if (!assets.has(`tests/assets/${arg.value}`))
      lift.refuse('asset', `${arg.value} (add it to sync.sh)`)
  }

  /** The fixture an identifier names, when it is the test function's own parameter. */
  function fixtureOf(path, node) {
    if (!test) return null
    const binding = path.scope.getBinding(node.name)
    if (binding?.kind !== 'param' || binding.scope.block !== root.node)
      return null
    return node.name
  }

  function reference(path) {
    const { node } = path
    const name = node.name
    const binding = path.scope.getBinding(name)
    if (binding === undefined) {
      if (NODE_ONLY.has(name)) lift.refuse('node', `uses ${name}`)
      if (SIGNAL_ONLY.has(name))
        lift.refuse('unsupported', `${name}: the signal option`)
      return
    }
    const within = (inner) =>
      inner.node.start >= root.node.start && inner.node.end <= root.node.end
    if (binding.kind === 'param' && binding.scope.block === root.node) {
      if (test && path.node !== binding.identifier) fixture(binding)
      return
    }
    if (within(binding.path)) return
    if (binding.kind === 'module') return imported(path, binding)
    const loop = binding.path.parentPath?.parentPath
    if (
      binding.path.isVariableDeclarator() &&
      loop?.isForOfStatement() &&
      !within(loop)
    ) {
      if (name in lift.test.scope.env) return
      return lift.refuse('shape', `uses the loop binding ${name}`)
    }
    helper(lift, lift.test.source, binding.path, !pageFunctions.has(node))
  }

  /** A test function's parameter: a fixture from its first, destructured one, or test info. */
  function fixture(binding) {
    const name = binding.identifier.name
    const fixtures = root.node.params[0]
    const { start, end } = binding.identifier
    if (fixtures === undefined || start < fixtures.start || end > fixtures.end)
      return lift.refuse('fixture', `uses its test info (${name})`)
    if (name in CASE_FIXTURES) lift.fixtures.add(name)
    if (PROVIDED.has(name)) return
    if (name in FIXTURE_VALUES) lift.constants.add(name)
    else lift.refuse('fixture', `uses the ${name} fixture`)
  }

  function imported(path, binding) {
    const from = binding.path.parentPath.node.source.value
    const name = binding.path.node.imported?.name
    if (from === './pageTest' && name === 'test') {
      const parent = path.parentPath
      if (parent.isMemberExpression() && parent.node.object === path.node)
        return
      return lift.refuse('shape', 'uses its test object as a value')
    }
    if (from === './pageTest' && name === 'expect') {
      const inWeb = webRanges.some(
        ([start, end]) => path.node.start >= start && path.node.end <= end,
      )
      if (!inWeb) lift.refuse('shape', 'uses expect outside an assertion')
      return
    }
    if (
      (from === './pageTest' && reexported.has(name)) ||
      from === '../config/utils'
    ) {
      const declaration = utilsExport(name)
      if (declaration === null)
        return lift.refuse('shape', `imports ${name} from ${from}`)
      helper(lift, utils(), declaration, true)
      return
    }
    lift.refuse('node', `imports ${from}`)
  }
}

/** Inlines a declaration the test reaches, and walks it when it runs in the runner. */
function helper(lift, source, path, runnerSide) {
  const statement = path.isVariableDeclarator() ? path.parentPath : path
  if (
    !statement.isVariableDeclaration() &&
    !statement.isFunctionDeclaration() &&
    !statement.isClassDeclaration()
  )
    return lift.refuse(
      'shape',
      `reaches ${source.code.slice(path.node.start, path.node.end).split('\n')[0]}`,
    )
  const key = `${source.path}:${statement.node.start}`
  const known = lift.helpers.get(key)
  if (known && (known.scanned || !runnerSide)) return
  lift.helpers.set(key, { source, node: statement.node, scanned: runnerSide })
  if (runnerSide) scan(lift, statement)
}

let utilsSource = null
function utils() {
  utilsSource ??= new Source(UTILS)
  return utilsSource
}

/** utils.ts's exported function named `name`, including pageTest.ts's re-export of it. */
function utilsExport(name) {
  let found = null
  for (const statement of utils().program.get('body')) {
    const declaration = statement.get('declaration')
    if (
      statement.isExportNamedDeclaration() &&
      declaration.node?.id?.name === name
    )
      found = declaration
  }
  return found
}

// ---- rendering a case ------------------------------------------------------------------

/** The in-case runtime, hoisted to the top of each case's function body. */
const RUNTIME = `function tier2Runtime(fixtures) {
  const expects = {}
  // A message's call log and stack frames are each runner's own (the page's V8, Node, QuickJS).
  const text = (value) =>
    value
      .split('\\nCall log:')[0]
      .split('\\n')
      .filter((line) => !/^\\s+at /.test(line))
      .join('\\n')
  const threw = (error) => ({
    threw: {
      name: String(error?.name ?? 'Error'),
      message: text(String(error?.message ?? error)),
    },
  })
  const shape = (value, depth) => {
    if (value === undefined) return '(undefined)'
    if (value === null || typeof value === 'boolean') return value
    if (typeof value === 'string') return text(value)
    if (typeof value === 'number') return Number.isFinite(value) ? value : String(value)
    if (typeof value === 'function') return '(function)'
    if (typeof value !== 'object') return String(value)
    if (depth > 8) return '(nested too deep)'
    if (Array.isArray(value)) return value.map((item) => shape(item, depth + 1))
    if (value instanceof Error) return threw(value)
    if (typeof value.asElement === 'function')
      return value.asElement() === null ? '(JSHandle)' : '(ElementHandle)'
    if (typeof value.evaluateAll === 'function') return '(Locator)'
    if (typeof value.mainFrame === 'function') return '(Page)'
    if (typeof value.parentFrame === 'function') return '(Frame)'
    if (ArrayBuffer.isView(value)) return \`(\${value.byteLength} bytes)\`
    const shaped = {}
    for (const key of Object.keys(value)) shaped[key] = shape(value[key], depth + 1)
    return shaped
  }
  return {
    server: {
      PREFIX: fixtures.url('upstream', 'b'),
      CROSS_PROCESS_PREFIX: fixtures.url('upstream', 'a'),
      EMPTY_PAGE: fixtures.url('upstream/empty.html', 'b'),
    },
    html: (markup) => \`data:text/html;charset=utf-8,\${encodeURIComponent(markup)}\`,
    /** toMatch's verdict: a string pattern is a substring, as in Jest. */
    matches: (value, pattern) =>
      typeof value !== 'string'
        ? shape(value, 0)
        : typeof pattern === 'string'
          ? value.includes(pattern)
          : pattern.test(value),
    /** Records what \`received\` gives (or throws) under the expect's line, as JSON. */
    record: (line, received) => settle(line, received, false),
    /** As record, then gives back what \`received\` gave, rejecting as it rejected. */
    pass: (line, received) => settle(line, received, true),
    done: (end) => ('threw' in end ? { ...expects, threw: threw(end.threw).threw } : expects),
  }

  function settle(line, received, passThrough) {
    let key = line
    for (let count = 2; key in expects; count++) key = \`\${line}#\${count}\`
    expects[key] = '(pending)'
    const put = (value) => {
      expects[key] = shape(value, 0)
      return value
    }
    const fail = (error) => {
      expects[key] = threw(error)
      if (passThrough) throw error
    }
    try {
      const value = received()
      return typeof value?.then === 'function' ? value.then(put, fail) : put(value)
    } catch (error) {
      return fail(error)
    }
  }
}`

function slug(titles) {
  const text = titles
    .join(' ')
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
  if (text.length <= 72) return text
  const cut = text.slice(0, 72)
  return cut.slice(0, cut.lastIndexOf('-') > 40 ? cut.lastIndexOf('-') : 72)
}

function needsLine(needs) {
  const units = [...needs.keys()].sort()
  const what = units
    .map((unit) => `${[...needs.get(unit)].sort().join(', ')} (${unit})`)
    .join('; ')
  return `${units.join(', ')}: ${what} not built yet`
}

/** The case file for one lifted test. */
function render(test, lift) {
  const { source } = test
  const lines = [
    `// Upstream: Playwright ${tag} ${source.path}:${test.line}`,
    `// Test: ${test.titles.join(' > ').replace(/\s+/g, ' ')}`,
    '// Lifted by tier2/convert.mjs. Rerun it instead of editing this file.',
  ]
  for (const note of lift.notes) lines.push(`// Note: ${note}`)
  if (lift.needs.size > 0) lines.push(`// @needs ${needsLine(lift.needs)}`)
  lines.push('const tier2 = tier2Runtime(fixtures)')
  for (const name of [...lift.fixtures].sort()) lines.push(CASE_FIXTURES[name])
  for (const name of [...lift.constants].sort())
    lines.push(
      `const ${name} = ${FIXTURE_VALUES[name] === undefined ? 'undefined' : JSON.stringify(FIXTURE_VALUES[name])}`,
    )
  lines.push(...test.scope.bindings)
  const helpers = [...lift.helpers.values()].sort(
    (a, b) =>
      (a.source === b.source ? 0 : a.source === utilsSource ? -1 : 1) ||
      a.node.start - b.node.start,
  )
  for (const { source: from, node } of helpers) lines.push(from.nodeText(node))
  const body = test.fn.node.body
  const text = source
    .text(body.start + 1, body.end - 1)
    .replace(/^[ \t]*\n/, '')
    .replace(/\n[ \t]*$/, '')
  lines.push(
    'return tier2.done(await harness.settle((async () => {',
    text,
    '})()))',
    '',
    RUNTIME,
    '',
  )
  const out = lines.join('\n')
  try {
    babelParse(
      `async function lifted(page, context, fixtures, harness) {\n${out}\n}`,
      'case.js',
      true,
    )
  } catch (error) {
    throw new Error(
      `${source.path}:${test.line}: the lifted case is not JavaScript: ${error.message}`,
    )
  }
  return out
}

// ---- main ------------------------------------------------------------------------------

const tests = []
for (const spec of specs) {
  const source = new Source(spec)
  collectScope(
    source,
    source.program.get('body'),
    { titles: [], skip: null, notes: [], env: {}, bindings: [] },
    tests,
  )
}

const outputs = new Map()
const skipped = []
const counts = { lifted: 0, runs: 0, needs: {}, skipped: {} }
const names = new Set()
for (const test of tests) {
  const lift = new Lift(test)
  if (lift.skip === null) scan(lift, test.fn, { test: true })
  const where = `${test.source.path}:${test.line}`
  const title = test.titles.join(' > ')
  const costly = SWEEP_COST.get(`${test.source.path} > ${title}`)
  if (costly !== undefined) lift.need('sweep-cost', costly)
  if (lift.skip !== null) {
    skipped.push({
      test: where,
      title,
      kind: lift.skip.kind,
      reason: lift.skip.detail,
    })
    counts.skipped[lift.skip.kind] = (counts.skipped[lift.skip.kind] ?? 0) + 1
    continue
  }
  const base = `${basename(test.source.path, '.spec.ts')}.${slug(test.titles)}`
  let name = base
  for (let count = 2; names.has(name); count++) name = `${base}-${count}`
  names.add(name)
  outputs.set(`${name}.js`, render(test, lift))
  counts.lifted++
  if (lift.needs.size === 0) counts.runs++
  for (const unit of lift.needs.keys())
    counts.needs[unit] = (counts.needs[unit] ?? 0) + 1
}
const skippedText = `${JSON.stringify(skipped, null, 2)}\n`

let stale = 0
const existing = new Set(
  (() => {
    try {
      return readdirSync(casesDir).filter((file) => file.endsWith('.js'))
    } catch {
      return []
    }
  })(),
)
if (check) {
  for (const [file, text] of outputs) {
    let disk = null
    try {
      disk = readFileSync(resolve(casesDir, file), 'utf8')
    } catch {}
    if (disk !== text) {
      stale++
      console.log(`stale    cases/tier2/${file}`)
    }
  }
  for (const file of existing)
    if (!outputs.has(file)) {
      stale++
      console.log(`extra    cases/tier2/${file}`)
    }
  let disk = null
  try {
    disk = readFileSync(skippedFile, 'utf8')
  } catch {}
  if (disk !== skippedText) {
    stale++
    console.log('stale    tier2/skipped.json')
  }
} else {
  mkdirSync(casesDir, { recursive: true })
  for (const file of existing)
    if (!outputs.has(file)) rmSync(resolve(casesDir, file))
  for (const [file, text] of outputs)
    writeFileSync(resolve(casesDir, file), text)
  writeFileSync(skippedFile, skippedText)
}

const byUnit = Object.entries(counts.needs)
  .sort()
  .map(([unit, count]) => `${count} ${unit}`)
  .join(', ')
const byKind = Object.entries(counts.skipped)
  .sort()
  .map(([kind, count]) => `${count} ${kind}`)
  .join(', ')
console.log(
  `${tests.length} upstream tests in ${specs.length} specs: ${counts.lifted} lifted (${counts.runs} run now; @needs ${byUnit || 'none'}), ${skipped.length} skipped (${byKind})`,
)
if (check && stale > 0) {
  console.log(`${stale} files differ; rerun node tier2/convert.mjs`)
  process.exit(1)
}
