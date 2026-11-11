/**
 * Stable case IDs for the pinned corpus. Scope, curation and reports name tests by ID
 * (`FI-005`), but the pinned Playwright release predates upstream's IDs in test titles
 * (microsoft/playwright#43485), so the spec sources stay untouched and `case-ids.json`
 * attaches an ID to each collected test by spec file and title path. Every run checks the map
 * against the collected cases, so a test can never fall back to an ID-like word in its title.
 */
import fs from 'node:fs'

const manifestPath = new URL('./case-ids.json', import.meta.url)
const keyOf = (file, title) => JSON.stringify([file, title])

export function loadCaseIds(file = manifestPath) {
  const manifest = JSON.parse(fs.readFileSync(file, 'utf8'))
  if (
    manifest?.schemaVersion !== 1 ||
    typeof manifest.pin !== 'string' ||
    !manifest.files ||
    typeof manifest.files !== 'object'
  )
    throw new Error('Invalid case ID schema')
  const ids = new Map()
  for (const [suite, titles] of Object.entries(manifest.files))
    for (const [title, id] of Object.entries(titles)) {
      if (typeof id !== 'string' || !/^[A-Z][A-Z0-9]*-\d+$/.test(id))
        throw new Error(`Invalid case ID for ${suite}: ${title}`)
      ids.set(keyOf(suite, title), id)
    }
  return { pin: manifest.pin, ids }
}

let pinned
/** The pinned ID of a collected test, or undefined for a title path the map does not know. */
export function pinnedCaseId(file, title) {
  pinned ??= loadCaseIds()
  return pinned.ids.get(keyOf(file, title))
}

/**
 * Collection is where the map meets the pinned sources. A missing or stale entry fails the run
 * here, before curation or scope can silently miss a test.
 */
export function verifyCaseIds(cases, revision, map = loadCaseIds()) {
  if (map.pin !== revision)
    throw new Error('Case ID map pin differs from the prepared corpus')
  const collected = new Set()
  for (const row of cases) {
    const key = keyOf(row.suite, row.title)
    if (!map.ids.has(key))
      throw new Error(
        `Collected test has no case ID: ${row.suite}: ${row.title}`,
      )
    collected.add(key)
  }
  for (const key of map.ids.keys())
    if (!collected.has(key))
      throw new Error(`Case ID names no collected test: ${key}`)
}
