/** Neo: Pure interpretation of Playwright Test JSON; raw attempts never replace the runner's verdict. */
import { stripVTControlCharacters } from 'node:util'
import { refusals } from './refusals.mjs'
import { clawRefusals } from './refusals-claw.mjs'

/** Neo: Stable report columns, including every exclusive failure class. */
export const verdicts = [
  'pass',
  'pass via expectLoop',
  'neo-lacks',
  'neo-differs',
  'needs-internals',
  'invalid-here',
  'harness-error',
  'skip',
]

export function normalizeSuite(file, corpus = []) {
  const normalized = file.replaceAll('\\', '/').replace(/^\.\//, '')
  const candidates = corpus
    .map((suite) => (typeof suite === 'string' ? suite : suite.file))
    .filter(
      (suite) =>
        normalized === suite ||
        normalized.endsWith(`/${suite}`) ||
        suite === `tests/${normalized}`,
    )
  if (candidates.length === 1) return candidates[0]
  const index = normalized.lastIndexOf('/tests/')
  if (index !== -1) return normalized.slice(index + 1)
  return /^(page|library)\//.test(normalized)
    ? `tests/${normalized}`
    : normalized
}

export function caseKey(suite, titles) {
  return JSON.stringify([suite, titles])
}

/** Duplicate titles across projects/repeats cannot be joined by the required identity. Fail visibly. */
export function flattenResults(report, corpus = []) {
  if (!Array.isArray(report.suites))
    throw new Error('results.json must contain a suites array')
  const cases = []
  const seen = new Set()
  function visit(suite, parents = [], inheritedFile) {
    const file = normalizeSuite(suite.file || inheritedFile || '', corpus)
    const isFileSuite = normalizeSuite(suite.title || '', corpus) === file
    const titles =
      isFileSuite || !suite.title ? parents : [...parents, suite.title]
    for (const spec of suite.specs || []) {
      const specFile = normalizeSuite(spec.file || file, corpus)
      const titlePath = [...titles, spec.title]
      const key = caseKey(specFile, titlePath)
      if (!specFile || typeof spec.title !== 'string')
        throw new Error('Case is missing its file or title')
      if (seen.has(key) || spec.tests?.length > 1)
        throw new Error(
          `Duplicate case identity; use one project and repeatEach=1: ${key}`,
        )
      seen.add(key)
      const test = spec.tests?.[0] || { results: [] }
      cases.push({
        key,
        suite: specFile,
        titles: titlePath,
        title: titlePath.join(' › '),
        caseId: spec.title.match(/\b[A-Z][A-Z0-9]*-\d+\b/)?.[0] || null,
        location: { file: specFile, line: spec.line, column: spec.column },
        test,
      })
    }
    for (const child of suite.suites || []) visit(child, titles, file)
  }
  for (const suite of report.suites) visit(suite)
  return cases
}

function clean(value) {
  return stripVTControlCharacters(String(value || '')).trim()
}

function firstLine(value) {
  return clean(value).split('\n')[0]
}

export function errorEvidence(error, fallback = null) {
  const message = clean(
    typeof error === 'string'
      ? error
      : error.message || error.value || error.stack,
  )
  const stack = clean(error.stack)
  let location = error.location
  if (!location || location.file?.startsWith('<')) {
    const sites = [
      ...`${message}\n${stack}`.matchAll(
        /(?:at |\()([^()\n]+\.spec\.[cm]?[jt]s):(\d+):(\d+)\)?/g,
      ),
    ]
    const site = sites[0]
    location = site
      ? { file: site[1].trim(), line: Number(site[2]), column: Number(site[3]) }
      : fallback
  }
  return {
    firstLine: firstLine(message),
    location: location || fallback,
    message,
    stack,
    snippet: clean(error.snippet),
  }
}

function errorsOf(test, location) {
  return (test.results || []).flatMap((result, attempt) => {
    // JSON's error is the first errors entry again, with a less useful formatted message.
    const errors = result.errors?.length
      ? result.errors
      : result.error
        ? [result.error]
        : []
    return errors.map((error) => ({
      ...errorEvidence(error, location),
      attempt,
      status: result.status,
    }))
  })
}

/** Neo: Only explicit infrastructure signals; ordinary test timeouts/closed-page errors are outcomes. */
// `Neo hosted runs keep at most`: the Node door's own handle caps (hosted-run only), not product behavior.
function isHarnessError(line) {
  if (isLoopError(line)) return false
  return /(?:worker process|test worker).*exited unexpectedly|Browser (?:did not (?:start|publish|own)|exited at startup)|Owned browser process group survived cleanup|Cannot revalidate browser group|(?:^|: )harness-error:|ERR_DLOPEN_FAILED|Cannot find module .*(?:neo_node|claw_node)|dlopen\([^\n]*claw_node|(?:browser|Browser) (?:process )?(?:crashed|disconnected unexpectedly)|browserType\.launch: (?:Executable doesn't exist|Failed to launch|Target .*closed|spawn )|connect ECONNREFUSED|EADDRINUSE|Timeout.*(?:setting up|tearing down).*"__(?:neo|claw)"|Neo hosted runs keep at most/i.test(
    line,
  )
}

function isLoopError(line) {
  return /^(?:(?:AssertionError(?: \[ERR_ASSERTION\])?|Error): )?expectLoop:/.test(
    line,
  )
}

function outcome(row) {
  const test = row.test
  const results = test.results || []
  if (test.status === 'skipped') return 'skip'
  if (!results.length) return 'harness-error'
  if (test.status === 'expected' || test.status === 'flaky') return 'pass'
  if (results.some((result) => result.status === 'interrupted'))
    return 'harness-error'
  if (test.status === 'unexpected') return 'fail'
  // Do not infer a green result from malformed/partial JSON lacking Playwright's status.
  return 'harness-error'
}

function ownerOf(expression) {
  if (/\.asElement\(/.test(expression)) return 'ElementHandle'
  if (/\.mainFrame\(|\.frames\(/.test(expression)) return 'Frame'
  if (/\.(?:locator|getBy\w+|frameLocator)\(/.test(expression)) return 'Locator'
  if (/\.context\(/.test(expression)) return 'BrowserContext'
  const device = expression.match(/^page\d*\.(keyboard|mouse|touchscreen)\./)
  if (device) return device[1][0].toUpperCase() + device[1].slice(1)
  const root = expression.replace(/^\(?await\s+/, '').split(/[.([]/)[0]
  if (/^(page\d*|popup|newPage|otherPage|opener|Page)$/.test(root))
    return 'Page'
  if (/^(context\d*|browserContext|newContext|BrowserContext)$/.test(root))
    return 'BrowserContext'
  if (/^(frame\d*|childFrame|mainFrame|Frame)$/.test(root)) return 'Frame'
  if (/^(locator\d*|Locator)$/.test(root)) return 'Locator'
  if (/^(elementHandle\d*|element\d*|handle\d*|ElementHandle)$/.test(root))
    return 'ElementHandle'
  if (/^(jsHandle|JSHandle)$/.test(root)) return 'JSHandle'
  if (/^(browser\d*|Browser)$/.test(root)) return 'Browser'
  if (/^(browserType|chromium|firefox|webkit|BrowserType)$/.test(root))
    return 'BrowserType'
  if (
    /^(request|Request|response|Response|route|Route|download|Download|keyboard|Keyboard|mouse|Mouse|touchscreen|Touchscreen|dialog|Dialog)$/.test(
      root,
    )
  )
    return root[0].toUpperCase() + root.slice(1)
  if (/^playwright\.(chromium|firefox|webkit)\./.test(expression))
    return 'BrowserType'
  return null
}

/** Playwright: `client/elementHandle.ts` `ElementHandle`'s public methods, minus `JSHandle`'s. */
const ELEMENT_HANDLE_METHODS = new Set(
  '$ $$ boundingBox check click contentFrame dblclick dispatchEvent fill focus getAttribute hover innerHTML innerText inputValue isChecked isDisabled isEditable isEnabled isHidden isVisible ownerFrame press screenshot scrollIntoViewIfNeeded selectOption selectText setChecked setInputFiles tap textContent type uncheck waitForElementState waitForSelector'.split(
    ' ',
  ),
)

function apiKey(expression) {
  const method = expression.match(/\.([\w$]+)(?:\(.*\))?$/)?.[1]
  // Neo's Page, Frame and Locator serve these names, so a test's own handle variable
  // (`div`, `button`, from `page.$`) missing one is an ElementHandle.
  const owner =
    ownerOf(expression) ||
    (/^[a-z]\w*\.[\w$]+$/.test(expression) && ELEMENT_HANDLE_METHODS.has(method)
      ? 'ElementHandle'
      : null)
  return owner && method ? `${owner}.${method}` : null
}

function internalGap(error) {
  const line = error.firstLine
  const explicit = line.match(/^(?:Error: )?needs Playwright internals:\s*(.+)/)
  if (explicit) {
    if (/\b(?:_expect|Frame\.expect)\b/.test(explicit[1]))
      return {
        key: apiKey(explicit[1]) || 'Locator._expect',
        expectOnly:
          /^(?:(?:Locator|Page|Frame)\.)?_expect$|^Frame\.expect$/.test(
            explicit[1],
          ),
      }
    return { key: explicit[1], expectOnly: false }
  }
  if (
    /\bFrame\.expect\b/.test(line) &&
    /Neo does not serve|not a function/.test(line)
  )
    return { key: 'Frame.expect', expectOnly: true }
  if (/\._expect is not a function/.test(line))
    return {
      key:
        apiKey(
          line.replace(/^TypeError: /, '').replace(/ is not a function.*$/, ''),
        ) || 'Locator._expect',
      expectOnly: true,
    }
  // Playwright's `expectTypes` refuses a receiver without `_apiName` (`Page`/`Locator`) before
  // the matcher runs; Phase 2's `_apiName` + `_expect` is the same seam.
  if (/can be only used with (?:Page or Locator|Locator) objects/.test(line))
    return { key: 'Page/Locator._apiName', expectOnly: true }
  // Tests that assert on a failed matcher's message fail on that assertion, whose received
  // text is the missing seam's TypeError. Only the received side counts, never the expected.
  if (/^Received[^\n]*\._expect is not a function/m.test(error.message || ''))
    return { key: 'Locator._expect', expectOnly: true }
  if (
    /\btoImpl\b|Neo Node harness does not provide Playwright internal fixture/.test(
      line,
    )
  )
    return {
      key: line.match(/internal fixture (\w+)/)?.[1] || 'toImpl',
      expectOnly: false,
    }
  if (/\b_options\b/.test(line))
    return { key: 'BrowserContext._options', expectOnly: false }
  // The matcher strips its own stack. Require both the precise JS error and its toHaveURL call site.
  if (
    /Cannot read properties of undefined \(reading ['"]baseURL['"]\)/.test(
      line,
    ) &&
    /\.toHaveURL\(/.test(`${error.message}\n${error.snippet}`)
  )
    return { key: 'BrowserContext._options.baseURL', expectOnly: false }
  if (/\b_wrapApiCall\b/.test(line))
    return {
      key:
        apiKey(
          line.replace(/^TypeError: /, '').replace(/ is not a function.*$/, ''),
        ) || 'Playwright._wrapApiCall',
      expectOnly: false,
    }
  if (/\b(?:launchOptions|launchPersistent|startRemoteServer)\b/.test(line))
    return { key: 'Playwright.launchOptions', expectOnly: false }
  return null
}

function observedGap(error, row, engine = 'neo') {
  const line = error.firstLine
  // `page.request` (APIRequestContext) is absent on Neo's objects, so its methods read undefined.
  const read = line.match(
    /Cannot read properties of undefined \(reading '([\w$]+)'\)/,
  )
  if (engine === 'claw' && read) {
    const evidence = `${error.snippet || ''}\n${error.message || ''}`
    const namespace = evidence.match(
      /\b(playwright\.(?:chromium|firefox|webkit|request|selectors)|browserType)(?:\.|\[)/,
    )
    if (namespace) return { key: namespace[1], missing: true }
  }
  const owner =
    read &&
    `${error.snippet || ''}\n${error.message || ''}`.match(
      new RegExp(`\\b(page|context)\\d*\\.request\\.${read[1]}\\b`),
    )
  if (owner)
    return {
      key: `${owner[1] === 'page' ? 'Page' : 'BrowserContext'}.request`,
      missing: true,
    }
  const missing = line.match(/^(?:TypeError: )?(.+) is not a function\.?$/)
  const expression = missing?.[1]
  let key = expression ? apiKey(expression) : null
  if (!key) {
    const api = line.match(
      /\b(?:page|browserContext|context|browser|browserType|locator|elementHandle|jsHandle|frame|request|response|route|download|keyboard|mouse|touchscreen|dialog|Page|BrowserContext|Browser|BrowserType|Locator|ElementHandle|JSHandle|Frame|Request|Response|Route)\.[\w$]+/,
    )
    key = api && apiKey(api[0])
  }
  if (!key) {
    const api = `${error.stack}\n${error.message}`.match(/\bat (\w+\.\w+)\s*\(/)
    key = api && apiKey(api[1])
  }
  if (!key)
    key = `${row.suite.split('/').at(-1)}: ${line.replace(/^(?:\w*Error: )/, '') || 'unexpected outcome'}`
  if (/context browser\.newContext created/.test(line))
    key += ' (created context)'
  const option = line.match(
    /Neo does not support the "([^"]+)" option of a new context/,
  )
  if (option) key = `Browser.newContext (${option[1]})`
  else {
    const detail = line.match(
      /Neo does not support (signal|exposeFunctions|steps|a clickCount|a predicate function as url|waitUntil: "networkidle")/,
    )
    if (detail) key += ` (${detail[1]})`
    if (/on child frames yet/.test(line)) key += ' (child frame)'
  }
  return { key, missing: Boolean(expression && apiKey(expression)) }
}

function classifyLoopError(error, neo, engine, refusalList) {
  const readFailure = error.firstLine.split('; read failed: ')[1]
  const gap =
    readFailure &&
    observedGap({ ...error, firstLine: readFailure }, neo, engine)
  const missing =
    gap?.missing ||
    (readFailure &&
      refusalList.some((refusal) => refusal.pattern.test(readFailure)))
  return {
    verdict: missing ? 'neo-lacks' : 'neo-differs',
    gapKey:
      gap?.key ||
      `expectLoop.${error.firstLine.match(/expectLoop: (?:not\.)?(\w+)/)?.[1]}`,
    reason: error.firstLine,
  }
}

function nonFailure(row) {
  const skipped = row.outcome === 'skip'
  return {
    ...row,
    verdict: skipped
      ? 'skip'
      : row.expectLoopChecks.length
        ? 'pass via expectLoop'
        : 'pass',
    reason:
      skipped && row.skipReasons.length
        ? row.skipReasons.join('; ')
        : `Playwright Test: ${row.playwrightStatus}`,
  }
}

export function classifyCase(neo, reference, { engine = 'neo' } = {}) {
  const refusalList = engine === 'claw' ? clawRefusals : refusals
  const errors = errorsOf(neo.test, neo.location)
  const referenceErrors = reference
    ? errorsOf(reference.test, reference.location)
    : []
  const neoOutcome = outcome(neo)
  const referenceOutcome = reference ? outcome(reference) : 'missing'
  const annotations = [
    ...(neo.test.annotations || []),
    ...(neo.test.results || []).flatMap((result) => result.annotations || []),
  ]
  const expectLoopChecks = [
    ...new Set(
      annotations
        .filter((a) => a.type === 'expectLoop')
        .map((a) => a.description),
    ),
  ]
  const skipReasons = [
    ...new Set(
      annotations
        .filter((a) => a.type === 'skip' && a.description)
        .map((a) => a.description),
    ),
  ]
  const row = {
    key: neo.key,
    suite: neo.suite,
    titles: neo.titles,
    title: neo.title,
    caseId: neo.caseId,
    location: neo.location,
    outcome: neoOutcome,
    playwrightStatus: neo.test.status || null,
    expectedStatus: neo.test.expectedStatus || null,
    attempts: (neo.test.results || []).map((result) => ({
      status: result.status,
      retry: result.retry || 0,
    })),
    referenceOutcome,
    referenceStatus: reference?.test.status || null,
    evidence: errors,
    referenceEvidence: referenceErrors,
    gapKey: null,
    wouldPassWithExpect: false,
    expectLoopChecks,
    skipReasons,
  }
  if (neoOutcome === 'pass' || neoOutcome === 'skip') return nonFailure(row)
  const infrastructure = errors.find((error) => isHarnessError(error.firstLine))
  const referenceInfrastructure =
    referenceOutcome === 'fail' &&
    referenceErrors.find((error) => isHarnessError(error.firstLine))
  if (
    neoOutcome === 'harness-error' ||
    infrastructure ||
    referenceOutcome === 'harness-error' ||
    referenceInfrastructure
  ) {
    const evidence = infrastructure || referenceInfrastructure || errors[0]
    return {
      ...row,
      outcome: 'harness-error',
      verdict: 'harness-error',
      gapKey: 'Harness',
      reason:
        evidence?.firstLine ||
        'Interrupted, incomplete, or missing Playwright test result',
    }
  }
  if (referenceOutcome === 'fail')
    return {
      ...row,
      verdict: 'invalid-here',
      gapKey: referenceErrors.length
        ? observedGap(referenceErrors[0], reference).key
        : 'Playwright.expectedStatus',
      reason:
        referenceErrors[0]?.firstLine ||
        'Real Playwright also reports an unexpected outcome',
    }
  const loopError = errors.find((error) => isLoopError(error.firstLine))
  if (loopError)
    return { ...row, ...classifyLoopError(loopError, neo, engine, refusalList) }
  const internals = errors.map(internalGap)
  const internal = internals.find(Boolean)
  if (internal)
    return {
      ...row,
      verdict: 'needs-internals',
      gapKey: internal.key,
      reason: errors[internals.indexOf(internal)].firstLine,
      wouldPassWithExpect:
        internals.length > 0 && internals.every((gap) => gap?.expectOnly),
    }
  for (const error of errors) {
    const gap = observedGap(error, neo, engine)
    if (
      gap.missing ||
      refusalList.some((refusal) => refusal.pattern.test(error.firstLine))
    )
      return {
        ...row,
        verdict: 'neo-lacks',
        gapKey: gap.key,
        reason: error.firstLine,
      }
  }
  return {
    ...row,
    verdict: 'neo-differs',
    gapKey: errors.length
      ? observedGap(errors[0], neo, engine).key
      : 'Playwright.expectedStatus',
    reason:
      errors[0]?.firstLine ||
      `Unexpected outcome (expected ${neo.test.expectedStatus}, got ${neo.test.results?.at(-1)?.status})`,
  }
}

export function classifyRuns(neo, reference, corpus = [], options = {}) {
  const referenceCases = new Map(
    flattenResults(reference, corpus).map((row) => [row.key, row]),
  )
  return flattenResults(neo, corpus).map((row) =>
    classifyCase(row, referenceCases.get(row.key), options),
  )
}

export function countVerdicts(rows) {
  const counts = Object.fromEntries(verdicts.map((verdict) => [verdict, 0]))
  for (const row of rows) counts[row.verdict]++
  return counts
}

/** The bridge stores the same full title path, including describes; case ids alone are not unique. */
export function compareBridge(rows, bridge, corpus = []) {
  const byKey = new Map()
  for (const entry of bridge) {
    const [file, titles] = JSON.parse(entry.key)
    if (
      !Array.isArray(titles) ||
      !titles.every((title) => typeof title === 'string')
    )
      throw new Error(`Invalid bridge identity: ${entry.key}`)
    const key = caseKey(normalizeSuite(file, corpus), titles)
    if (byKey.has(key)) throw new Error(`Duplicate bridge identity: ${key}`)
    byKey.set(key, entry)
  }
  const noVerdict = new Set(['BridgeLimit', 'CleanupBlur', 'HarnessError'])
  const matrix = {}
  let matched = 0
  let recovered = 0
  let noVerdictSelected = 0
  for (const row of rows) {
    const prior = byKey.get(row.key)
    const label = prior?.verdict || 'No bridge match'
    matrix[label] ||= countVerdicts([])
    matrix[label][row.verdict]++
    if (prior) matched++
    if (noVerdict.has(prior?.verdict)) {
      noVerdictSelected++
      if (row.outcome === 'pass' || row.outcome === 'fail') recovered++
    }
  }
  return {
    matrix,
    matched,
    unmatched: rows.length - matched,
    noVerdictTotal: bridge.filter((entry) => noVerdict.has(entry.verdict))
      .length,
    noVerdictSelected,
    recovered,
    bridgeTotal: bridge.length,
  }
}
