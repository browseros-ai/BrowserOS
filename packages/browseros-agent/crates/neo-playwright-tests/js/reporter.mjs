/** Discovery and raw runner outcomes. Expected failures never become compatibility passes. */
import fs from 'node:fs'
import path from 'node:path'
import { corpus, sha256 } from './prepare.mjs'

export function identity(test, root) {
  const titles = [test.title]
  for (
    let suite = test.parent;
    suite && suite.type !== 'file';
    suite = suite.parent
  )
    if (suite.title) titles.unshift(suite.title)
  const file = path.relative(root, test.location.file).split(path.sep).join('/')
  const key = JSON.stringify([file, titles])
  const slug = `${file
    .replace(/^tests\//, '')
    .replace(/\.spec\.ts$/, '')
    .replace(
      /[^a-zA-Z0-9]+/g,
      '_',
    )}::${titles.join('_').replace(/[^a-zA-Z0-9]+/g, '_')}`.toLowerCase()
  return { key, name: `${slug}__${sha256(key).slice(0, 12)}`, file, titles }
}

export default class Reporter {
  outcomes = []
  errors = []
  printsToStdio() {
    return false
  }
  async preprocess({ config, suite, testRun }) {
    this.root = path.dirname(config.configFile)
    const catalog = []
    const names = new Set()
    const keys = new Set()
    for (const test of suite.allTests()) {
      const entry = identity(test, this.root)
      const policy = corpus.suites.find(
        (candidate) => candidate.file === entry.file,
      )
      if (!policy?.include || (policy.title && test.title !== policy.title)) {
        testRun.exclude(test)
        continue
      }
      if (keys.has(entry.key) || names.has(entry.name))
        throw new Error(`Case identity collision ${entry.key}`)
      keys.add(entry.key)
      names.add(entry.name)
      let mode = 'default'
      for (let parent = test.parent; parent; parent = parent.parent)
        if (testRun.mode(parent) === 'serial') mode = 'serial'
      catalog.push({
        ...entry,
        feature: policy.feature || 'unclassified',
        upstreamId: test.id,
        location: {
          file: entry.file,
          line: test.location.line,
          column: test.location.column,
        },
        mode,
        annotations: test.annotations,
      })
      if (
        process.env.NEO_TESTS_CASE &&
        process.env.NEO_TESTS_CASE !== entry.key
      )
        testRun.exclude(test)
    }
    fs.writeFileSync(
      process.env.NEO_TESTS_CATALOG,
      JSON.stringify(catalog, null, 2),
    )
  }
  onTestEnd(test, result) {
    this.outcomes.push({
      ...identity(test, this.root),
      upstreamId: test.id,
      rawStatus: result.status,
      expectedStatus: test.expectedStatus,
      outcome: test.outcome(),
      annotations: test.annotations,
      errors: result.errors,
    })
  }
  onError(error) {
    this.errors.push(error)
  }
  onEnd(result) {
    if (process.env.NEO_TESTS_RESULT)
      fs.writeFileSync(
        process.env.NEO_TESTS_RESULT,
        JSON.stringify(
          {
            status: result.status,
            outcomes: this.outcomes,
            errors: this.errors,
          },
          null,
          2,
        ),
      )
  }
}
