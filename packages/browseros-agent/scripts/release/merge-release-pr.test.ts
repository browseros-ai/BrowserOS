import { describe, expect, it } from 'bun:test'
import { spawnSync } from 'node:child_process'
import {
  chmodSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const script = resolve(import.meta.dir, 'merge-release-pr.sh')
const head = '0123456789abcdef0123456789abcdef01234567'

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'merge-release-pr-'))
  const bin = join(root, 'gh')
  const state = join(root, 'state')
  const calls = join(root, 'calls')
  writeFileSync(state, 'OPEN')
  writeFileSync(calls, '')
  writeFileSync(
    bin,
    `#!/bin/sh
set -eu
case "$1:$2" in
  pr:view)
    printf '{"state":"%s","mergeStateStatus":"%s","headRefOid":"%s","isDraft":false,"statusCheckRollup":[]}\n' \
      "$(cat "$MERGE_TEST_STATE")" "${'$'}{MERGE_TEST_MERGE_STATE:-CLEAN}" "${'$'}{MERGE_TEST_HEAD}"
    ;;
  pr:merge)
    printf '%s\n' "$*" >> "$MERGE_TEST_CALLS"
    count="$(wc -l < "$MERGE_TEST_CALLS" | tr -d ' ')"
    if [ "$count" -lt "${'$'}{MERGE_TEST_SUCCEED_ON_CALL:-1}" ]; then
      exit 1
    fi
    printf 'MERGED' > "$MERGE_TEST_STATE"
    ;;
  *) exit 2 ;;
esac
`,
  )
  chmodSync(bin, 0o755)
  return { root, state, calls }
}

function run(
  test: ReturnType<typeof fixture>,
  env: Record<string, string> = {},
) {
  return spawnSync(script, ['https://example.test/pull/1', head], {
    env: {
      ...process.env,
      GH_TOKEN: 'token',
      GITHUB_REPOSITORY: 'example/repo',
      MERGE_TEST_CALLS: test.calls,
      MERGE_TEST_HEAD: head,
      MERGE_TEST_STATE: test.state,
      PATH: `${test.root}:${process.env.PATH}`,
      RELEASE_PR_MERGE_POLL_SECONDS: '0',
      ...env,
    },
    encoding: 'utf8',
  })
}

describe('merge-release-pr', () => {
  it('merges a clean pull request immediately with head protection', () => {
    const test = fixture()
    try {
      const result = run(test)
      expect(result.status, result.stderr || result.stdout).toBe(0)
      expect(result.stdout).toContain('Release PR merged')
      const calls = readFileSync(test.calls, 'utf8')
      expect(calls).toContain('--squash')
      expect(calls).toContain(`--match-head-commit ${head}`)
      expect(calls).not.toContain('--auto')
    } finally {
      rmSync(test.root, { recursive: true, force: true })
    }
  })

  it('falls back to auto-merge and retries transient failures', () => {
    const test = fixture()
    try {
      const result = run(test, {
        MERGE_TEST_SUCCEED_ON_CALL: '3',
        RELEASE_PR_MERGE_ATTEMPTS: '2',
      })
      expect(result.status, result.stderr || result.stdout).toBe(0)
      expect(result.stdout).toContain('not merged yet (1/2)')
      expect(readFileSync(test.calls, 'utf8')).toContain('--auto')
    } finally {
      rmSync(test.root, { recursive: true, force: true })
    }
  })

  it('refuses a changed pull request head', () => {
    const test = fixture()
    try {
      const result = run(test, {
        MERGE_TEST_HEAD: 'fedcba9876543210fedcba9876543210fedcba98',
      })
      expect(result.status).not.toBe(0)
      expect(result.stderr).toContain('Release PR head changed')
      expect(readFileSync(test.calls, 'utf8')).toBe('')
    } finally {
      rmSync(test.root, { recursive: true, force: true })
    }
  })
})
