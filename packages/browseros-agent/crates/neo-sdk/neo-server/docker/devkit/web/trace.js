// The dev kit's trace UI: claw-sdk's runs and operations as they happen, joined with the CLI
// run log (`dev.sh run` writes it; the gateway serves it as /dev/runs) on `requestId`.
//
// One state object and one reducer (design.md data shape 4). Everything that changes state is
// an event: claw-sdk's own `Event`s from /v1/events, and local ones (a page loaded, a click).
// `apply` never does I/O; it returns commands, `perform` runs them, and their results come
// back as events. So every state change can be read in one switch.

import { api, clock, duration, h } from '/kit/common.js'

const EVENT_TYPES = [
  'run.started',
  'run.ended',
  'operation.started',
  'operation.ended',
  'tab.closed',
  'lock.state',
  'takeover.state',
  'stream.reset',
]
const NEO_EVENTS_KEPT = 200
const RUN_PAGE = 100
/** Re-reads with `wait=10` until a run is complete (its screenshot work done); bounded. */
const COMPLETE_TRIES = 3
const STREAM_RETRY_MS = 3000
/** How long after a run ends its CLI record may still be on its way. */
const CLI_GRACE_MS = 10000

const state = {
  runs: new Map(), // runId -> RunSummary, from /v1/runs and run.* events
  nextCursor: null,
  details: new Map(), // runId -> Run, for runs that were selected
  rev: new Map(), // runId -> a counter bumped whenever its Run changes, so the detail re-renders only then
  cli: new Map(), // requestId -> CLI run record (design.md shape 2)
  shots: new Map(), // operationId -> {state: loading|ok|expired|missing|error, url?}
  selected: null,
  follow: true,
  filter: '',
  stream: 'connecting', // connecting | live | down
  neoEvents: [], // the last NEO_EVENTS_KEPT tab.*, lock.state, takeover.state events
  problem: null, // the last failed load, shown as a banner until a load succeeds
  open: new Set(), // long JSON blocks the reader expanded
  uiRev: 0,
}

const seqOf = (operationId) =>
  Number(operationId.slice(operationId.lastIndexOf('.') + 1))
const bump = (s, runId) => s.rev.set(runId, (s.rev.get(runId) ?? 0) + 1)

function matchesFilter(run, filter) {
  if (!filter) return true
  const hay = [
    run.id,
    run.requestId,
    run.taskId,
    run.status,
    ...(run.tabs ?? []),
  ]
    .join(' ')
    .toLowerCase()
  return filter
    .toLowerCase()
    .split(/\s+/)
    .every((word) => hay.includes(word))
}

/** The newest run the filter shows: what "follow newest" follows, so a task filter follows that task. */
function newestRunId(s) {
  let best = null
  for (const run of s.runs.values()) {
    if (!matchesFilter(run, s.filter)) continue
    if (
      !best ||
      run.startedAt > best.startedAt ||
      (run.startedAt === best.startedAt && run.id > best.id)
    )
      best = run
  }
  return best?.id ?? null
}

/**
 * Operations keyed by id, in start order (`seq`). When both copies of one operation are known,
 * an ended copy beats a running one, and otherwise the incoming copy wins: an event can be newer
 * than a fetched run, and a re-fetched run carries `hasScreenshot` once its capture is done.
 */
function mergeOperations(existing, incoming) {
  const byId = new Map(existing.map((op) => [op.id, op]))
  for (const op of incoming) {
    const old = byId.get(op.id)
    byId.set(op.id, !old || op.outcome || !old.outcome ? op : old)
  }
  return [...byId.values()].sort((a, b) => seqOf(a.id) - seqOf(b.id))
}

/** A run summary that counts `op`: its operation count and its tabs (ascending, as claw-sdk keeps them). */
function withOperation(run, op) {
  const tabs =
    op.tabId === undefined || run.tabs?.includes(op.tabId)
      ? run.tabs
      : [...(run.tabs ?? []), op.tabId].sort((a, b) => a - b)
  return {
    ...run,
    tabs,
    operationCount: Math.max(run.operationCount ?? 0, seqOf(op.id)),
  }
}

const loadRun = (run, tries = 0) => ({
  do: 'loadRun',
  id: run.id,
  // An ended run that is not complete is still taking its screenshots: hold the read until done.
  wait: run.status !== 'running' && !run.complete ? 10 : 0,
  tries,
})

/**
 * Screenshot reads for a run's operations. `hasScreenshot` is final once the run is complete,
 * and false when no image is stored. Once complete, each operation with a tab is checked:
 * 404 means no image was kept, and 410 means retention deleted it.
 */
function shotLoads(s, run) {
  const commands = []
  for (const op of run.operations ?? []) {
    const ask = op.hasScreenshot || (run.complete && op.tabId !== undefined)
    if (ask && !s.shots.has(op.id)) {
      s.shots.set(op.id, { state: 'loading' })
      commands.push({ do: 'loadShot', id: op.id })
    }
  }
  return commands
}

function select(s, runId, commands) {
  s.selected = runId
  const summary = s.runs.get(runId)
  let run = s.details.get(runId)
  if (!run && summary) {
    // A stub, so operation events that arrive before the fetch lands are kept (and merged).
    run = { ...summary, operations: [], stub: true }
    s.details.set(runId, run)
    bump(s, runId)
  }
  if (!run) return
  if (run.stub || !run.complete) commands.push(loadRun(run))
  commands.push(...shotLoads(s, run))
  if (run.requestId && !s.cli.has(run.requestId))
    commands.push({ do: 'loadCli' })
}

/** Applies one event to the state; returns the loads it calls for. Never does I/O. */
function apply(s, event) {
  const commands = []
  switch (event.type) {
    // ----- claw-sdk's events (/v1/events) -----
    case 'run.started':
    case 'run.ended': {
      const run = event.run
      s.runs.set(run.id, run)
      const known = s.details.get(run.id)
      if (known) {
        s.details.set(run.id, {
          ...known,
          ...run,
          operations: known.operations,
        })
        bump(s, run.id)
      }
      if (
        event.type === 'run.started' &&
        s.follow &&
        matchesFilter(run, s.filter)
      )
        select(s, run.id, commands)
      else if (known) commands.push(loadRun(run))
      // The CLI appends its record once it has the reply, a moment after run.ended; cli.loaded
      // reads again while the selected run's record is still missing.
      if (event.type === 'run.ended' && run.requestId)
        commands.push({ do: 'loadCli', delayMs: 300 })
      break
    }
    case 'operation.started':
    case 'operation.ended': {
      // The run's own counters move only with run.* events; keep them current between.
      const op = event.operation
      const summary = s.runs.get(op.runId)
      if (summary) s.runs.set(op.runId, withOperation(summary, op))
      const run = s.details.get(op.runId)
      if (run) {
        s.details.set(op.runId, {
          ...withOperation(run, op),
          operations: mergeOperations(run.operations, [op]),
        })
        bump(s, op.runId)
      }
      break
    }
    case 'tab.closed':
    case 'lock.state':
    case 'takeover.state':
      s.neoEvents.push(event)
      if (s.neoEvents.length > NEO_EVENTS_KEPT)
        s.neoEvents.splice(0, s.neoEvents.length - NEO_EVENTS_KEPT)
      break
    case 'stream.reset': {
      // Events were missed (Neo restarted, or this page fell behind): read the state again.
      commands.push({ do: 'loadRuns' }, { do: 'loadCli' })
      const selected = s.selected && s.details.get(s.selected)
      if (selected) commands.push({ ...loadRun(selected), wait: 0 })
      break
    }

    // ----- local events -----
    case 'stream.status':
      s.stream = event.status
      break
    case 'runs.loaded':
      for (const run of event.items) s.runs.set(run.id, run)
      s.nextCursor = event.nextCursor
      s.problem = null
      if (s.follow && !s.selected && newestRunId(s))
        select(s, newestRunId(s), commands)
      break
    case 'run.loaded': {
      const run = event.run
      const known = s.details.get(run.id)
      const { operations, ...summary } = run
      s.details.set(run.id, {
        ...run,
        operations: mergeOperations(known?.operations ?? [], operations),
      })
      s.runs.set(run.id, { ...s.runs.get(run.id), ...summary })
      bump(s, run.id)
      s.problem = null
      if (
        run.status !== 'running' &&
        !run.complete &&
        event.tries < COMPLETE_TRIES
      ) {
        commands.push(loadRun(run, event.tries + 1))
      }
      if (run.id === s.selected)
        commands.push(...shotLoads(s, s.details.get(run.id)))
      break
    }
    case 'cli.loaded': {
      s.cli = new Map(
        event.records
          .filter((r) => r && r.requestId)
          .map((r) => [r.requestId, r]),
      )
      const run = s.selected && s.runs.get(s.selected)
      if (
        run?.requestId &&
        !s.cli.has(run.requestId) &&
        run.endedAt &&
        Date.now() - run.endedAt < CLI_GRACE_MS
      ) {
        commands.push({ do: 'loadCli', delayMs: 1000 })
      }
      break
    }
    case 'shot.loaded':
      s.shots.set(event.id, event.shot)
      if (s.selected) bump(s, s.selected)
      break
    case 'load.failed':
      s.problem = event.message
      break
    case 'select':
      // Picking an older run means "let me read this": stop jumping to new runs.
      if (event.runId !== newestRunId(s)) s.follow = false
      select(s, event.runId, commands)
      break
    case 'follow':
      s.follow = event.on
      if (event.on && newestRunId(s)) select(s, newestRunId(s), commands)
      break
    case 'filter':
      s.filter = event.text
      if (s.follow && newestRunId(s)) select(s, newestRunId(s), commands)
      break
    case 'toggle':
      if (event.open) s.open.add(event.key)
      else s.open.delete(event.key)
      s.uiRev += 1
      break
  }
  return commands
}

// ---------- effects ----------

const inflight = new Set()
let cliTimer = null

async function perform(command) {
  try {
    switch (command.do) {
      case 'loadRuns': {
        const cursor = command.cursor
          ? `&cursor=${encodeURIComponent(command.cursor)}`
          : ''
        const page = await api(`/v1/runs?limit=${RUN_PAGE}${cursor}`)
        // Only the first page's cursor matters until "older" is pressed; then the latest page's.
        dispatch({
          type: 'runs.loaded',
          items: page.items,
          nextCursor: page.nextCursor ?? null,
        })
        break
      }
      case 'loadRun': {
        const key = `${command.id}?${command.wait}`
        if (inflight.has(key)) return
        inflight.add(key)
        let run
        try {
          run = await api(
            `/v1/runs/${encodeURIComponent(command.id)}${command.wait ? `?wait=${command.wait}` : ''}`,
          )
        } finally {
          inflight.delete(key)
        }
        // Dispatched after the key is gone: a run still incomplete schedules the same read
        // again, synchronously, and must not find this one in flight.
        dispatch({ type: 'run.loaded', run, tries: command.tries })
        break
      }
      case 'loadCli':
        // Coalesced: a burst of run.ended events reads the log once.
        if (cliTimer) return
        cliTimer = setTimeout(async () => {
          cliTimer = null
          try {
            dispatch({ type: 'cli.loaded', records: await api('/dev/runs') })
          } catch (error) {
            dispatch({
              type: 'load.failed',
              message: `/dev/runs: ${error.message}`,
            })
          }
        }, command.delayMs ?? 0)
        break
      case 'loadShot': {
        const res = await fetch(
          `/v1/operations/${encodeURIComponent(command.id)}/screenshot`,
        )
        let shot
        if (res.ok)
          shot = { state: 'ok', url: URL.createObjectURL(await res.blob()) }
        else {
          const body = await res.json().catch(() => ({}))
          shot =
            res.status === 410
              ? { state: 'expired' }
              : {
                  state: res.status === 404 ? 'missing' : 'error',
                  note: body.message,
                }
        }
        dispatch({ type: 'shot.loaded', id: command.id, shot })
        break
      }
    }
  } catch (error) {
    dispatch({
      type: 'load.failed',
      message: `${command.do}: ${error.message}`,
    })
  }
}

let renderQueued = false
function dispatch(event) {
  for (const command of apply(state, event)) perform(command)
  if (!renderQueued) {
    renderQueued = true
    requestAnimationFrame(() => {
      renderQueued = false
      render()
    })
  }
}

/**
 * /v1/events through the gateway. The browser's own reconnect sends `Last-Event-ID`, and
 * claw-sdk resumes or answers `stream.reset`. A refused reconnect (the gateway's 502 while the
 * container restarts) closes the EventSource for good, so a fresh one is opened; it has no
 * `Last-Event-ID`, so the page re-reads its state itself, as on a reset.
 */
function openStream(afterFailure) {
  dispatch({ type: 'stream.status', status: 'connecting' })
  const source = new EventSource('/v1/events')
  source.onopen = () => {
    dispatch({ type: 'stream.status', status: 'live' })
    if (afterFailure) {
      afterFailure = false
      dispatch({ type: 'stream.reset' })
    }
  }
  for (const type of EVENT_TYPES) {
    source.addEventListener(type, (message) =>
      dispatch(JSON.parse(message.data)),
    )
  }
  source.onerror = () => {
    if (source.readyState === EventSource.CLOSED) {
      dispatch({ type: 'stream.status', status: 'down' })
      setTimeout(() => openStream(true), STREAM_RETRY_MS)
    } else {
      dispatch({ type: 'stream.status', status: 'connecting' })
    }
  }
}

// ---------- rendering ----------

const $ = (id) => document.getElementById(id)

const TONE = {
  ok: 'ok',
  done: 'ok',
  error: 'err',
  running: 'info',
  timeout: 'warn',
  cancelled: 'warn',
  unknown: 'warn',
}
const badge = (text, tone = TONE[text] ?? '') =>
  text === undefined || text === null
    ? null
    : h('span', { class: `badge ${tone}` }, text)

function renderList() {
  const runs = [...state.runs.values()]
    .filter((run) => matchesFilter(run, state.filter))
    .sort((a, b) => b.startedAt - a.startedAt || (b.id > a.id ? 1 : -1))
  const rows = runs.map((run) =>
    h(
      'div',
      {
        class: `run${run.id === state.selected ? ' sel' : ''}`,
        onclick: () => dispatch({ type: 'select', runId: run.id }),
      },
      badge(run.status),
      h(
        'span',
        { class: 'who', title: run.requestId ?? run.id },
        run.requestId ?? run.id,
      ),
      h('span', { class: 'when' }, clock(run.startedAt)),
      h(
        'span',
        { class: 'meta' },
        run.taskId
          ? h(
              'span',
              { class: 'task', title: run.taskId },
              'task ',
              h('b', null, run.taskId),
            )
          : null,
        h('span', null, `${run.operationCount} ops`),
        run.endedAt
          ? h('span', null, duration(run.endedAt - run.startedAt))
          : h('span', null, 'running'),
        run.error
          ? h(
              'span',
              { style: 'color:var(--err)' },
              `${run.error.owner}/${run.error.code}`,
            )
          : null,
      ),
    ),
  )
  $('list').replaceChildren(
    ...(rows.length
      ? rows
      : [
          h(
            'div',
            { class: 'empty' },
            state.runs.size ? 'No run matches the filter.' : 'No runs yet.',
          ),
        ]),
    ...(state.nextCursor
      ? [
          h(
            'div',
            { class: 'more' },
            h(
              'button',
              {
                onclick: () =>
                  perform({ do: 'loadRuns', cursor: state.nextCursor }),
              },
              'Load older runs',
            ),
          ),
        ]
      : []),
  )
}

/** Short JSON inline; long JSON as a collapsed block whose open state survives re-renders. */
function jsonBlock(key, value, truncated) {
  if (value === undefined) return h('span', { class: 'muted' }, '(none)')
  const text = JSON.stringify(value, null, 2)
  const note = truncated ? badge('truncated', 'warn') : null
  if (text.length <= 800 && text.split('\n').length <= 24)
    return h('div', null, h('pre', null, text), note)
  const preview = JSON.stringify(value).slice(0, 140)
  return h(
    'details',
    {
      open: state.open.has(key),
      ontoggle: (event) => {
        if (event.target.open !== state.open.has(key))
          dispatch({ type: 'toggle', key, open: event.target.open })
      },
    },
    h(
      'summary',
      null,
      `${preview}${preview.length >= 140 ? '...' : ''}  (${text.length} chars)`,
    ),
    h('pre', null, text),
    note,
  )
}

function errorLine(error) {
  if (!error) return null
  return h(
    'div',
    { class: 'error' },
    badge(`owner: ${error.owner}`, 'err'),
    ' ',
    h('b', null, error.code),
    error.hostCode ? ` (${error.hostCode})` : '',
    `: ${error.message}`,
    error.retryable ? [' ', badge('retryable')] : null,
    error.operationId
      ? h('span', { class: 'muted' }, `  (from op ${seqOf(error.operationId)})`)
      : null,
  )
}

function field(label, content) {
  return h(
    'div',
    { class: 'field' },
    h('span', { class: 'k' }, label),
    h('div', null, content),
  )
}

function thumb(op, run) {
  const shot = state.shots.get(op.id)
  // No read yet: a run still taking its screenshots, or an operation with no tab (see shotLoads).
  if (!shot)
    return h(
      'div',
      { class: 'thumb' },
      run.complete && !op.hasScreenshot ? 'no screenshot' : '',
    )
  if (shot.state === 'loading')
    return h('div', { class: 'thumb' }, 'loading...')
  if (shot.state === 'ok') {
    return h(
      'div',
      { class: 'thumb' },
      h('img', {
        src: shot.url,
        alt: `after ${op.name}`,
        onclick: () => lightbox(shot.url),
      }),
    )
  }
  const words =
    { expired: 'expired', missing: 'no screenshot' }[shot.state] ??
    `error: ${shot.note ?? ''}`
  return h('div', { class: 'thumb' }, words)
}

function renderOperation(op, run, depth) {
  const failed = op.outcome === 'error'
  return h(
    'div',
    {
      class: `op${failed ? ' failed' : ''}`,
      style: depth ? `margin-left:${depth * 18}px` : null,
    },
    h(
      'div',
      { style: 'min-width:0' },
      h(
        'div',
        { class: 'top' },
        h('span', { class: 'seq' }, seqOf(op.id)),
        h('span', { class: 'name' }, op.name),
        badge(op.kind),
        badge(op.effect, op.effect === 'write' ? 'info' : ''),
        op.tabId !== undefined
          ? h('span', { class: 'muted mono' }, `tab ${op.tabId}`)
          : null,
        op.outcome ? badge(op.outcome) : badge('running', 'info'),
        op.delivery
          ? badge(`delivery: ${op.delivery}`, TONE[op.delivery] ?? '')
          : null,
        h('span', { class: 'ms' }, `+${op.startedAt - run.startedAt} ms`),
        op.durationMs !== undefined
          ? h('span', { class: 'ms' }, duration(op.durationMs))
          : null,
        op.attempt ? h('span', { class: 'ms' }, `attempt ${op.attempt}`) : null,
      ),
      h(
        'div',
        { class: 'body' },
        op.page?.url
          ? h(
              'div',
              {
                class: 'muted mono',
                style:
                  'font-size:11px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap',
              },
              `${op.page.title ? `${op.page.title}  ` : ''}${op.page.url}`,
            )
          : null,
        errorLine(op.error),
        field('args', jsonBlock(`${op.id}:args`, op.args, op.argsTruncated)),
        op.result !== undefined || op.outcome === 'ok'
          ? field(
              'result',
              jsonBlock(`${op.id}:result`, op.result, op.resultTruncated),
            )
          : null,
        op.notes?.length
          ? field(
              'notes',
              h(
                'ul',
                { class: 'notes' },
                op.notes.map((line) => h('li', null, line)),
              ),
            )
          : null,
      ),
    ),
    thumb(op, run),
  )
}

function depths(operations) {
  const parent = new Map(operations.map((op) => [op.id, op.parentId]))
  const depth = (id, seen = 0) =>
    parent.get(id) && seen < 20 ? 1 + depth(parent.get(id), seen + 1) : 0
  return new Map(operations.map((op) => [op.id, depth(op.id)]))
}

function cliText(response) {
  return (response?.content ?? [])
    .filter((part) => part.type === 'text')
    .map((part) => part.text)
    .join('\n')
}

const MAKE_ONE = `dev.sh run --task demo 'await page.goto("https://example.com"); return await page.title()'`

function renderDetail() {
  const id = state.selected
  if (!id || !state.runs.has(id)) {
    return h(
      'div',
      { class: 'empty' },
      state.runs.size
        ? 'Pick a run on the left.'
        : 'No runs yet. Make one from packages/browseros-agent/crates/neo-sdk/neo-server/docker:',
      state.runs.size ? null : h('pre', null, MAKE_ONE),
    )
  }
  const run = state.details.get(id) ?? state.runs.get(id)
  const cli = run.requestId ? state.cli.get(run.requestId) : undefined
  const operations = run.operations ?? []
  const depthOf = depths(operations)
  const meta = cli?.response?._meta?.['com.browseros.neo/run']

  let code
  if (run.code !== undefined) code = [h('pre', null, run.code)]
  else if (cli?.code !== undefined) {
    code = [
      run.codeWithheld === 'unavailable'
        ? h(
            'div',
            { class: 'notice warn', style: 'margin-bottom:6px' },
            'Neo restarted before saving this run’s code.',
          )
        : null,
      h(
        'div',
        { class: 'muted', style: 'margin-bottom:4px' },
        'from the CLI log',
      ),
      h('pre', null, cli.code),
    ]
  } else if (run.codeWithheld === 'unavailable')
    code = [
      h(
        'div',
        { class: 'notice warn' },
        'Neo restarted before saving this run’s code.',
      ),
    ]
  else if (run.status === 'running')
    code = [
      h(
        'span',
        { class: 'muted' },
        'claw-sdk stores the code when the run ends.',
      ),
    ]
  else
    code = [
      h('span', { class: 'muted' }, run.stub ? 'loading...' : '(no code)'),
    ]

  return h(
    'div',
    null,
    h(
      'div',
      { class: 'head' },
      h('span', { class: 'id' }, run.id),
      badge(run.status),
      run.effects
        ? badge(`effects: ${run.effects}`, TONE[run.effects] ?? '')
        : null,
      run.status !== 'running' && !run.complete
        ? badge('screenshots pending', 'info')
        : null,
      h('span', { class: 'spacer', style: 'flex:1' }),
      h('button', { onclick: () => exportRun(run.id) }, 'Export JSON'),
    ),
    h(
      'div',
      { class: 'facts' },
      run.taskId ? h('span', null, 'task ', h('b', null, run.taskId)) : null,
      run.requestId
        ? h('span', null, 'request ', h('b', null, run.requestId))
        : null,
      h('span', null, 'started ', h('b', null, clock(run.startedAt))),
      run.endedAt
        ? h(
            'span',
            null,
            'took ',
            h('b', null, duration(run.endedAt - run.startedAt)),
          )
        : null,
      h(
        'span',
        null,
        'tabs ',
        h('b', null, (run.tabs ?? []).join(', ') || 'none'),
      ),
      h('span', null, 'ops ', h('b', null, String(run.operationCount))),
      run.traceGaps
        ? h('span', null, 'trace gaps ', h('b', null, String(run.traceGaps)))
        : null,
    ),
    h('section', null, h('h2', null, 'Code'), code),
    h(
      'section',
      null,
      h('h2', null, 'Result'),
      errorLine(run.error),
      'result' in run
        ? jsonBlock(`${run.id}:result`, run.result, run.resultTruncated)
        : run.error
          ? null
          : h(
              'span',
              { class: 'muted' },
              run.status === 'running' ? 'running...' : '(none)',
            ),
      run.unknown?.length
        ? h(
            'div',
            { class: 'notice warn', style: 'margin-top:6px' },
            'Sent, never confirmed: ',
            run.unknown
              .map((u) => `${u.name} (op ${seqOf(u.operationId)})`)
              .join(', '),
          )
        : null,
    ),
    run.requestId
      ? h(
          'section',
          null,
          h(
            'h2',
            null,
            'CLI',
            h(
              'span',
              { class: 'muted', style: 'text-transform:none;letter-spacing:0' },
              'what the MCP client got',
            ),
          ),
          cli
            ? [
                cli.transportError
                  ? h(
                      'div',
                      { class: 'notice err' },
                      `transport error: ${cli.transportError}`,
                    )
                  : null,
                cli.response
                  ? h(
                      'div',
                      { style: 'display:flex;flex-direction:column;gap:5px' },
                      field(
                        'reply',
                        h(
                          'div',
                          null,
                          cli.response.isError ? badge('isError', 'err') : null,
                          h('pre', null, cliText(cli.response) || '(no text)'),
                        ),
                      ),
                      meta
                        ? field(
                            '_meta',
                            h(
                              'span',
                              { class: 'mono' },
                              `status ${meta.status}, effects ${meta.effects}, unknown ${meta.unknown?.length ?? 0}`,
                            ),
                          )
                        : null,
                    )
                  : null,
                field(
                  'timing',
                  h(
                    'span',
                    { class: 'mono' },
                    `${duration(cli.timing?.durationMs)} round trip${cli.timeoutMs ? `, timeout ${cli.timeoutMs} ms` : ''}`,
                  ),
                ),
              ]
            : h(
                'span',
                { class: 'muted' },
                'No CLI record for this request id (yet). Runs from other MCP clients have none.',
              ),
        )
      : null,
    h(
      'section',
      null,
      h('h2', null, `Operations (${operations.length})`),
      operations.length
        ? operations.map((op) => renderOperation(op, run, depthOf.get(op.id)))
        : h(
            'span',
            { class: 'muted' },
            run.stub ? 'loading...' : 'No operations.',
          ),
    ),
  )
}

function renderEvents() {
  const recent = state.neoEvents.slice(-30).reverse()
  const describe = (event) => {
    switch (event.type) {
      case 'lock.state':
        return event.lock
          ? `${event.lock.id} reads=${event.lock.reads} ${event.lock.releasedAt ? 'released' : `until ${clock(event.lock.expiresAt)}`}`
          : ''
      case 'takeover.state':
        return event.takeover
          ? `${event.takeover.id} ${event.takeover.state}${event.takeover.releaseReason ? ` (${event.takeover.releaseReason})` : ''}`
          : ''
      default:
        return ''
    }
  }
  $('events').replaceChildren(
    h('b', null, 'Neo events'),
    ...(recent.length
      ? recent.map((event) =>
          h(
            'span',
            { class: 'ev' },
            h('span', { class: 'muted' }, clock(event.at)),
            ' ',
            badge(event.type, 'info'),
            event.tabId !== undefined ? ` tab ${event.tabId} ` : ' ',
            describe(event),
          ),
        )
      : [
          h(
            'span',
            { class: 'muted' },
            'none yet (tab.closed, lock.state, takeover.state)',
          ),
        ]),
  )
}

let detailSignature = ''
function render() {
  const words = {
    live: ['ok', 'live'],
    connecting: ['warn', 'connecting'],
    down: ['err', 'down: retrying'],
  }[state.stream]
  $('stream').replaceChildren(
    h('span', { class: `dot ${words[0]}` }),
    h('span', { class: 'muted' }, words[1]),
  )
  $('follow').checked = state.follow
  $('banner').replaceChildren(
    state.problem
      ? h(
          'div',
          { class: 'notice err', style: 'margin:8px 12px' },
          state.problem,
        )
      : '',
  )
  renderList()
  // The detail is rebuilt only when what it shows changed, so text selection and scroll survive
  // events about other runs.
  const signature = `${state.selected}|${state.rev.get(state.selected)}|${state.cli.size}|${state.uiRev}|${state.runs.size === 0}`
  if (signature !== detailSignature) {
    detailSignature = signature
    $('detail').replaceChildren(renderDetail())
  }
  renderEvents()
}

function lightbox(url) {
  const box = h(
    'div',
    { class: 'lightbox', onclick: () => box.remove() },
    h('img', { src: url }),
  )
  document.body.append(box)
}

/** design.md decision 10: the run with its screenshots inlined, and its CLI record, as one file. */
async function exportRun(runId) {
  try {
    const [run, records] = await Promise.all([
      api(`/v1/runs/${encodeURIComponent(runId)}?include=screenshots`),
      api('/dev/runs'),
    ])
    const cli =
      (run.requestId && records.find((r) => r?.requestId === run.requestId)) ||
      null
    const url = URL.createObjectURL(
      new Blob([JSON.stringify({ run, cli }, null, 2)], {
        type: 'application/json',
      }),
    )
    const link = h('a', { href: url, download: `claw-run-${runId}.json` })
    document.body.append(link)
    link.click()
    link.remove()
    setTimeout(() => URL.revokeObjectURL(url), 30000)
  } catch (error) {
    dispatch({ type: 'load.failed', message: `export: ${error.message}` })
  }
}

$('follow').addEventListener('change', (event) =>
  dispatch({ type: 'follow', on: event.target.checked }),
)
$('filter').addEventListener('input', (event) =>
  dispatch({ type: 'filter', text: event.target.value.trim() }),
)
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape') document.querySelector('.lightbox')?.remove()
})

perform({ do: 'loadRuns' })
perform({ do: 'loadCli' })
openStream(false)
