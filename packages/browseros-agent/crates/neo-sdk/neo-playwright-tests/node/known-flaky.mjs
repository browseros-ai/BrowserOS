/**
 * Neo: Tests the CI gate does not judge because they flake. `known-flaky.tsv` is edited by hand in
 * a pull request, one test per line with why it flakes and where its fix is tracked; the gate
 * reports each one's outcome on every run, never fails a run over it in either direction, and
 * keeps it off the expected list. Nothing writes this file: the update command only reads it.
 * It is not scope (scope.json): these tests still count in every score.
 */
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import { testKey } from './expected.mjs'

const COLUMNS = ['id', 'file', 'title', 'reason', 'tracked']

export function knownFlakyPath() {
  return fileURLToPath(new URL('./known-flaky.tsv', import.meta.url))
}

/** Every field is required: an entry without a reason or a tracked fix is a test nobody owns. */
export function parseKnownFlaky(text) {
  const entries = new Map()
  text.split('\n').forEach((line, index) => {
    if (!line || line.startsWith('#')) return
    const fields = line.split('\t')
    const [id, file, title, reason, tracked] = fields
    if (
      fields.length !== COLUMNS.length ||
      fields.some((field) => !field.trim()) ||
      !/^tests\/(page|library)\/[^/]+\.spec\.ts$/.test(file)
    )
      throw new Error(
        `known-flaky.tsv line ${index + 1} is not "${COLUMNS.join('<tab>')}", each filled in`,
      )
    const key = testKey(file, title)
    if (entries.has(key))
      throw new Error(`known-flaky.tsv names ${file} › ${title} twice`)
    entries.set(key, {
      id: id === '-' ? null : id,
      file,
      title,
      reason,
      tracked,
    })
  })
  return entries
}

/** The checkout's list; no file means no known flaky tests. */
export function readKnownFlaky(file = knownFlakyPath()) {
  return fs.existsSync(file)
    ? parseKnownFlaky(fs.readFileSync(file, 'utf8'))
    : new Map()
}
