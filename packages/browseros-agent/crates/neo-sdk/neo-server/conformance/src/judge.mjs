/** Pure verdict pairing. A transport failure, unproven provenance or a broken oracle is never
 * evidence of Neo compatibility. Exception listings are exact, reviewed observations, not filters. */
const environmentCodes = new Set([
  'browser_disconnected',
  'browser_unavailable',
  'cdp_timeout',
  'shutting_down',
  'trace_unavailable',
])
const refusal = (error) =>
  error &&
  ((error.code === 'method_unavailable' && error.owner === 'script') ||
    (error.code === 'unsupported' && error.owner === 'neo'))
const firstError = (record) =>
  record.errors?.[0] || record.softErrors?.[0] || {}
const matches = (record, outcome) =>
  record.status === outcome.status &&
  (outcome.code ?? null) === (firstError(record).code ?? null) &&
  new RegExp(outcome.message).test(firstError(record).message || '')
// Translate only recognized failure paths across the native and protocol error vocabularies.
function failureKind({ name, message = '', code }) {
  if (
    name === 'TimeoutError' ||
    code === 'timeout' ||
    /\bTimeout \d+ms exceeded\b/i.test(message)
  )
    return 'timeout'
  if (/Frame locators are not allowed inside composite locators/i.test(message))
    return 'composite-frame-validation'
  if (
    code === 'strict_mode_violation' ||
    /strict mode violation/i.test(message)
  )
    return 'strict-mode'
  if (
    code === 'element_detached' ||
    /(?:not attached to the DOM|element was detached)/i.test(message)
  )
    return 'detached'
  if (
    /\bstate: (?:expected one of|invalid type|unknown variant)/i.test(message)
  )
    return 'invalid-state-option'
  if (code === 'no_match') return code
}

// Native Playwright and the Neo protocol can decorate the same exception differently.
// Compare the operation when both identify it, and keep the exception's actual name strict.
const operationHeading = /^([\w.$]+\.[\w$]+):\s*/
const operation = ({ message = '' }) => operationHeading.exec(message)?.[1]
const diagnosticText = ({ message }) =>
  message
    .split(/\n(?:[ \t]+at |Call log:)/, 1)[0]
    .replace(operationHeading, '')
    .replace(/^InvalidSelectorError: (?=Error while parsing selector )/, '')
    .trim()
const refusalApi = (error) => error.api || operation(error)

const matchingRefusal = (record, error) =>
  refusal(error) &&
  record.refusals?.find(
    (entry) =>
      entry.code === error.code &&
      entry.owner === error.owner &&
      entry.message === error.message,
  )

function diagnose(native, neo) {
  const p = native.diagnostics || []
  const n = neo.diagnostics || []
  if (p.length !== n.length)
    return { mismatch: 'diagnostic assertion count differs' }
  let rejected
  for (const [i, left] of p.entries()) {
    const right = n[i]
    const { error: a, matched } = left
    const { error: b, matched: neoMatched } = right
    const fields = ['line', 'column', 'matcher', 'kind', 'rejects']
    if (fields.some((key) => left[key] !== right[key]) || !a || !b)
      return {
        mismatch: `diagnostic assertion at line ${left.line} lacks a matching exception`,
      }

    // A run can contain unrelated caught refusals. Require the captured exception to
    // identify the traced refusal, then continue checking the remaining assertions.
    const nativeOp = operation(a),
      neoOp = operation(b)
    const traced = matchingRefusal(neo, b)
    if (traced && (!nativeOp || nativeOp === refusalApi(traced))) {
      rejected ??= traced
      continue
    }

    const ak = failureKind(a),
      bk = failureKind(b)
    // Auxiliary codes (e.g. filesystem ENOENT and protocol invalid_input) are not
    // exception types. Identical diagnostic text can establish the same path across them.
    const samePath =
      diagnosticText(a) === diagnosticText(b) ||
      (ak && ak === bk) ||
      (left.kind === 'throw' &&
        !ak &&
        !bk &&
        !a.code &&
        !b.code &&
        !nativeOp &&
        !neoOp)
    if (
      a.name !== b.name ||
      (nativeOp && neoOp && nativeOp !== neoOp) ||
      (ak && bk && ak !== bk) ||
      (!matched && !neoMatched) ||
      !samePath
    )
      return {
        mismatch: `diagnostic failure identity differs at line ${left.line}: ${ak || a.name} vs ${bk || b.name}`,
      }
  }
  return { rejected }
}

export function classify(
  row,
  native,
  neo,
  { deviations = [], oracleExceptions = [], unsupported = [] } = {},
) {
  const result = {
    id: row.id,
    spec: row.spec,
    line: row.line,
    programFile: row.programFile,
    native,
    neo,
    gating: false,
  }
  const finish = (classification, reason, gating = false) => ({
    ...result,
    classification,
    reason,
    gating,
  })
  if (row.class === 'excluded' || !row.selected)
    return finish(
      !row.selected
        ? 'not-selected'
        : row.kind === 'harness-unavailable'
          ? 'harness-unavailable'
          : `excluded:${row.kind}`,
      row.reason,
    )
  for (const [side, record] of [
    ['native', native],
    ['neo', neo],
  ]) {
    if (!record || record.transport || record.harnessError)
      return finish(
        'harness-error',
        `${side}: ${record?.transport?.error || record?.harnessError || 'missing record'}`,
        true,
      )
    const p = record.provenance
    if (
      record.id !== row.id ||
      !p?.programSha256 ||
      p.side !== side ||
      !p.browserVersion ||
      !['passed', 'failed', 'timedOut', 'skipped'].includes(record.status) ||
      !['passed', 'failed', 'skipped'].includes(record.expectedStatus) ||
      !Number.isInteger(record.assertions?.called)
    )
      return finish('harness-error', `${side}: invalid record/provenance`, true)
    if (side === 'native' && p.playwrightCore !== '1.64.1')
      return finish('harness-error', 'oracle version missing', true)
    if (
      side === 'neo' &&
      (!p.neo?.runId ||
        !p.neo.binarySha256 ||
        !p.neo.gitSha ||
        !Number.isInteger(p.neo.opCount) ||
        !p.neo.traceVerified)
    )
      return finish('harness-error', 'Neo trace provenance missing', true)
    if (
      side === 'neo' &&
      row.fixtures?.includes('page') &&
      record.status !== 'skipped' &&
      !firstError(record).harnessUnavailable &&
      p.neo.opCount < 1
    )
      return finish('harness-error', 'page test has no traced operations', true)
  }
  if (
    native.provenance.programSha256 !== neo.provenance.programSha256 ||
    native.provenance.browserVersion !== neo.provenance.browserVersion
  )
    return finish(
      'harness-error',
      'program bytes/browser versions differ',
      true,
    )
  const p = firstError(native),
    n = firstError(neo)
  if (p.harnessUnavailable || n.harnessUnavailable)
    return p.harnessUnavailable && n.harnessUnavailable
      ? finish(
          'harness-unavailable',
          `${p.harnessUnavailable}; ${n.harnessUnavailable}`,
        )
      : finish('harness-error', 'asymmetric harness unavailability', true)
  if ([p, n].some((e) => e.owner === 'neo' && environmentCodes.has(e.code)))
    return finish('harness-error', 'environment', true)
  if (
    native.status === 'skipped' &&
    neo.status === 'skipped' &&
    JSON.stringify(native.skip) === JSON.stringify(neo.skip)
  )
    return finish('skipped', native.skip?.reason)
  if (
    native.status === 'skipped' ||
    neo.status === 'skipped' ||
    native.expectedStatus !== neo.expectedStatus
  )
    return finish('fail', 'skip/expected-status differs', true)
  if (native.status !== native.expectedStatus) {
    const exception = oracleExceptions.find((e) => e.test === row.id)
    return finish(
      'oracle-fail',
      exception && matches(native, exception.native)
        ? exception.reason
        : 'unlisted or changed oracle failure',
      !(exception && matches(native, exception.native)),
    )
  }
  const unsupportedVerdict = (error) => {
    const api = refusalApi(error)
    return finish('unsupported', api, !unsupported.includes(api))
  }
  // An uncaught refusal explains the terminal failure directly. A caught one can
  // explain an assertion only when there is no independent failure in the pair.
  if (neo.status !== neo.expectedStatus && refusal(n))
    return unsupportedVerdict(matchingRefusal(neo, n) || n)
  if (
    neo.status === neo.expectedStatus &&
    native.assertions.called === neo.assertions.called
  ) {
    const { mismatch, rejected } = diagnose(native, neo)
    if (mismatch) return finish('fail', mismatch, true)
    if (rejected && neo.status === 'passed') return unsupportedVerdict(rejected)
    return finish('pass')
  }
  const deviation = deviations.find((e) => e.test === row.id)
  if (deviation && matches(neo, deviation.neo))
    return finish('deviation', `${deviation.reason} (${deviation.decision})`)
  const global =
    /(?:ReferenceError:\s*)?([A-Za-z_$][\w$]*) is not defined/.exec(
      n.message || '',
    )
  if (global) return finish('fail', `unsupported global ${global[1]}`, true)
  if (neo.status === neo.expectedStatus)
    return finish(
      'fail:assertion-count',
      `${native.assertions.called} vs ${neo.assertions.called}`,
      true,
    )
  return finish(
    'fail',
    `${neo.status}: ${n.code || n.name || ''}: ${n.message || ''}`,
    true,
  )
}
export function judge({
  inventory,
  native,
  neo,
  repeat = 1,
  deviations = [],
  oracleExceptions = [],
  unsupported = [],
}) {
  const runErrors = []
  const expected = new Set(
    inventory.tests
      .filter((row) => row.selected && row.class !== 'excluded')
      .map((row) => row.id),
  )
  const byAttempt = (side, records) => {
    const index = new Map()
    for (const record of records) {
      if (
        !expected.has(record.id) ||
        !Number.isInteger(record.attempt) ||
        record.attempt < 1 ||
        record.attempt > repeat
      ) {
        runErrors.push(
          `${side}: unexpected attempt ${record.id} #${record.attempt}`,
        )
        continue
      }
      const key = JSON.stringify([record.id, record.attempt])
      if (index.has(key)) {
        runErrors.push(
          `${side}: duplicate attempt ${record.id} #${record.attempt}`,
        )
        index.set(key, undefined)
      } else index.set(key, record)
    }
    return index
  }
  const ps = byAttempt('native', native),
    ns = byAttempt('neo', neo)
  const results = inventory.tests.flatMap((row) => {
    const attempts = Array.from(
      { length: expected.has(row.id) ? repeat : 1 },
      (_, i) => {
        const key = JSON.stringify([row.id, i + 1])
        return {
          ...classify(row, ps.get(key), ns.get(key), {
            deviations,
            oracleExceptions,
            unsupported,
          }),
          attempt: i + 1,
        }
      },
    )
    const flaky = new Set(attempts.map((r) => r.classification)).size > 1
    return attempts.map((r) => ({ ...r, flaky }))
  })
  const listingErrors = []
  for (const [list, classification] of [
    [deviations, 'deviation'],
    [oracleExceptions, 'oracle-fail'],
  ])
    for (const entry of list) {
      const matching = results.filter((r) => r.id === entry.test)
      const r = matching[0]
      // A filtered run validates only entries in its inventory. Full runs detect deleted ids.
      if (!r) {
        if (!inventory.filtered)
          listingErrors.push(`unknown listing: ${entry.test}`)
      } else if (
        matching.some((r) => r.classification !== classification || r.gating)
      )
        listingErrors.push(`stale/changed listing: ${entry.test}`)
    }
  const counts = {}
  for (const r of results)
    counts[r.classification] = (counts[r.classification] || 0) + 1
  return {
    ok:
      !runErrors.length &&
      !listingErrors.length &&
      !results.some((r) => r.gating),
    runErrors,
    counts,
    listingErrors,
    messageDiffs: results.flatMap((r) =>
      (r.neo?.messageDiffs || []).map((diff) => ({ id: r.id, ...diff })),
    ),
    results,
  }
}
export function markdown(report) {
  const error = (record) =>
    record?.transport?.error ||
    firstError(record || {}).message ||
    record?.status ||
    'missing'
  const describe = (result) =>
    `- **${result.classification}** ${result.id} (${result.spec}:${result.line})${result.flaky ? ' [flaky]' : ''}\n  ${result.reason || ''}\n  Program: ${result.programFile || 'not generated'}\n  Native: ${error(result.native).split('\n')[0]}\n  Neo: ${error(result.neo).split('\n')[0]}\n  Neo run: ${result.neo?.provenance?.neo?.runId || 'none'}`
  return (
    `# Playwright conformance\n\nGate: ${report.ok ? 'PASS' : 'FAIL'}\n\n${Object.entries(
      report.counts,
    )
      .map(([name, count]) => `- ${name}: ${count}`)
      .join(
        '\n',
      )}\n\n${[...(report.runErrors || []), ...report.listingErrors].join('\n')}\n\n` +
    report.results
      .filter((r) => !['pass', 'not-selected'].includes(r.classification))
      .map(describe)
      .join('\n') +
    '\n'
  )
}
