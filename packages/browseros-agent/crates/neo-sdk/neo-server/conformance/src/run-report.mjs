/** Rejudges persisted evidence against the run's requested attempts and terminal state.
 * Calibration records have their own directory and cannot masquerade as suite attempts. */
import {
  existsSync,
  readdirSync,
  readFileSync,
  renameSync,
  writeFileSync,
} from 'node:fs'
import { join } from 'node:path'
import { judge, markdown } from './judge.mjs'

const read = (file) => JSON.parse(readFileSync(file, 'utf8'))
const json = (file, value) =>
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`)

export function persistState(dir, state) {
  const file = join(dir, 'run-state.json')
  const staging = `${file}.tmp`
  json(staging, state)
  renameSync(staging, file)
}

export function writeReport(dir, settings) {
  let report = {
    ok: false,
    counts: {},
    results: [],
    listingErrors: [],
    messageDiffs: [],
  }
  let state
  try {
    state = read(join(dir, 'run-state.json'))
    if (
      state.schema !== 1 ||
      !Number.isSafeInteger(state.repeat) ||
      state.repeat < 1 ||
      !['run', 'selftest'].includes(state.mode) ||
      !['both', 'native', 'neo'].includes(state.side) ||
      !['running', 'completed', 'failed', 'interrupted'].includes(state.status)
    )
      throw new Error('invalid persisted run state')
    if (state.mode === 'selftest' && state.status === 'completed') {
      const calibration = read(join(dir, 'calibration.json'))
      if (
        !calibration.closedPort ||
        !calibration.results.length ||
        !calibration.results.every((row) => row.correct)
      )
        throw new Error('stored calibration is incomplete')
      report.ok = true
      report.counts = { 'calibration-as-expected': calibration.results.length }
      report.calibrations = calibration.results
    } else if (state.mode === 'run') {
      const records = (side) =>
        readdirSync(join(dir, side))
          .filter(
            (file) => file.endsWith('.json') && !file.endsWith('.trace.json'),
          )
          .map((file) => read(join(dir, side, file)))
      report = judge({
        inventory: read(join(dir, 'inventory.json')),
        repeat: state.repeat,
        native: records('native'),
        neo: records('neo'),
        ...settings,
      })
    }
    report.state = state.status
    report.phase = state.phase
    // The terminal state remains authoritative even after every expected pair was written.
    const failureFile = join(dir, 'failure.json')
    const failure = existsSync(failureFile)
      ? read(failureFile).error
      : state.failure
    if (state.status !== 'completed' || failure)
      throw new Error(failure || `run did not complete: ${state.status}`)
    if (state.side !== 'both') {
      report.ok = false
      report.side = state.side
      report.note = 'single-side diagnostic; no paired conformance verdict'
    }
  } catch (error) {
    report.ok = false
    report.failure = state?.failure || String(error)
    report.runErrors = [...(report.runErrors || []), report.failure]
    report.counts['harness-error'] = (report.counts['harness-error'] || 0) + 1
  }
  json(join(dir, 'report.json'), report)
  writeFileSync(join(dir, 'report.md'), markdown(report))
  return report
}
