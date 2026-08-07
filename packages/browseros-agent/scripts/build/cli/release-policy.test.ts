import { describe, expect, test } from 'bun:test'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

import {
  assertIncrementingRelease,
  compareReleaseVersions,
  ensureAnnotatedTag,
  ensureTagReachableFromDefaultBranch,
  parseCliReleaseTag,
  selectPreviousCliReleaseTag,
} from './release-policy'

const repoRoot = resolve(import.meta.dir, '../../../../..')
const cliReleaseWorkflow = readFileSync(
  resolve(repoRoot, '.github/workflows/release-cli.yml'),
  'utf8',
)

function cliGenerateReleaseNotesStep(): string {
  const start = cliReleaseWorkflow.indexOf('- name: Generate release notes')
  const end = cliReleaseWorkflow.indexOf('- name: Create GitHub release')
  expect(start).toBeGreaterThanOrEqual(0)
  expect(end).toBeGreaterThan(start)
  return cliReleaseWorkflow.slice(start, end)
}

function cliCreateGithubReleaseStep(): string {
  const start = cliReleaseWorkflow.indexOf('- name: Create GitHub release')
  const end = cliReleaseWorkflow.indexOf(
    '- name: Verify GitHub release assets are public',
  )
  expect(start).toBeGreaterThanOrEqual(0)
  expect(end).toBeGreaterThan(start)
  return cliReleaseWorkflow.slice(start, end)
}

describe('parseCliReleaseTag', () => {
  test('parses cli release tags', () => {
    expect(parseCliReleaseTag('cli/v0.2.3')).toEqual({
      tag: 'cli/v0.2.3',
      version: '0.2.3',
    })
  })

  test('rejects non-cli and non-strict tags', () => {
    for (const tag of [
      'browseros-cli-v0.2.3',
      'cli/0.2.3',
      'cli/v0.2',
      'cli/v01.2.3',
      'cli/v0.2.3-rc1',
      'server/v0.2.3',
    ]) {
      expect(() => parseCliReleaseTag(tag)).toThrow('cli/vX.Y.Z')
    }
  })
})

describe('compareReleaseVersions', () => {
  test('orders strict release versions', () => {
    expect(compareReleaseVersions('0.2.3', '0.2.2')).toBe(1)
    expect(compareReleaseVersions('0.2.2', '0.2.2')).toBe(0)
    expect(compareReleaseVersions('0.2.1', '0.2.2')).toBe(-1)
    expect(compareReleaseVersions('1.0.0', '0.9.9')).toBe(1)
  })

  test('rejects loose or prerelease versions', () => {
    expect(() => compareReleaseVersions('v0.2.3', '0.2.2')).toThrow(
      'strict release version',
    )
    expect(() => compareReleaseVersions('0.2.3-rc1', '0.2.2')).toThrow(
      'strict release version',
    )
  })
})

describe('assertIncrementingRelease', () => {
  test('allows releases newer than production latest', () => {
    expect(() => assertIncrementingRelease('0.2.3', '0.2.2')).not.toThrow()
  })

  test('rejects equal or lower releases', () => {
    expect(() => assertIncrementingRelease('0.2.2', '0.2.2')).toThrow(
      'must be greater',
    )
    expect(() => assertIncrementingRelease('0.2.1', '0.2.2')).toThrow(
      'must be greater',
    )
  })

  test('allows repair reruns when production latest already points at the same tag', () => {
    expect(() =>
      assertIncrementingRelease('0.2.3', '0.2.3', {
        tag: 'cli/v0.2.3',
        latestTag: 'cli/v0.2.3',
      }),
    ).not.toThrow()
    expect(() =>
      assertIncrementingRelease('0.2.3', '0.2.3', {
        tag: 'cli/v0.2.3',
        latestTag: 'browseros-cli-v0.2.3',
      }),
    ).toThrow('must be greater')
  })
})

describe('selectPreviousCliReleaseTag', () => {
  test('selects the latest earlier tag across new and legacy CLI tags', () => {
    expect(
      selectPreviousCliReleaseTag(
        [
          'browseros-cli-v0.2.0',
          'browseros-cli-v0.2.2',
          'cli/v0.0.1',
          'cli/v01.9.9',
          'browseros-cli-v0.1.0-rc1',
          'agent-extension-v1.0.0',
        ],
        '0.2.3',
      ),
    ).toBe('browseros-cli-v0.2.2')
  })

  test('prefers new cli tags when versions tie', () => {
    expect(
      selectPreviousCliReleaseTag(
        ['browseros-cli-v0.2.2', 'cli/v0.2.2'],
        '0.2.3',
      ),
    ).toBe('cli/v0.2.2')
  })

  test('returns empty string when there is no previous CLI release tag', () => {
    expect(
      selectPreviousCliReleaseTag(['agent-extension-v1.0.0'], '0.2.3'),
    ).toBe('')
  })
})

describe('ensureTagReachableFromDefaultBranch', () => {
  test('accepts default-branch tags and rejects tags only reachable from another commit', () => {
    const tmp = mkdtempSync(join(tmpdir(), 'cli-release-policy-'))
    try {
      const remote = join(tmp, 'origin.git')
      const repo = join(tmp, 'repo')
      git(tmp, ['init', '--bare', remote])
      git(tmp, ['clone', remote, repo])
      git(repo, ['config', 'user.email', 'test@example.com'])
      git(repo, ['config', 'user.name', 'Test User'])
      git(repo, ['checkout', '-b', 'main'])

      writeFileSync(join(repo, 'README.md'), 'initial\n')
      git(repo, ['add', 'README.md'])
      git(repo, ['commit', '-m', 'initial'])
      git(repo, ['tag', '-a', 'cli/v0.2.3', '-m', 'browseros-cli v0.2.3'])
      git(repo, ['push', 'origin', 'main', '--tags'])

      const reachableCommit = git(repo, ['rev-parse', 'cli/v0.2.3^{commit}'])
      expect(
        ensureTagReachableFromDefaultBranch(repo, 'cli/v0.2.3', 'main'),
      ).toBe(reachableCommit)
      expect(() => ensureAnnotatedTag(repo, 'cli/v0.2.3')).not.toThrow()

      git(repo, ['checkout', '-b', 'side'])
      writeFileSync(join(repo, 'side.txt'), 'side\n')
      git(repo, ['add', 'side.txt'])
      git(repo, ['commit', '-m', 'side'])
      git(repo, ['tag', '-a', 'cli/v0.2.4', '-m', 'browseros-cli v0.2.4'])
      git(repo, ['push', 'origin', 'cli/v0.2.4'])

      expect(() =>
        ensureTagReachableFromDefaultBranch(repo, 'cli/v0.2.4', 'main'),
      ).toThrow('not reachable')

      git(repo, ['checkout', 'main'])
      git(repo, ['tag', 'cli/v0.2.5'])

      expect(() => ensureAnnotatedTag(repo, 'cli/v0.2.5')).toThrow(
        'annotated tag',
      )
    } finally {
      rmSync(tmp, { recursive: true, force: true })
    }
  })
})

describe('release-cli workflow changelog cap', () => {
  test('caps only the isolated changelog before appending install instructions', () => {
    const step = cliGenerateReleaseNotesStep()

    expect(step).toContain('CHANGELOG_FILE="/tmp/release-changelog.md"')
    expect(step).toContain('NOTES_FILE="/tmp/release-notes.md"')
    expect(step).toContain(
      'node packages/browseros-agent/scripts/release/cap-release-changelog.mjs',
    )
    expect(step).toContain('--max-entries 15')
    expect(step).toContain('--previous-tag "$PREV_TAG"')
    expect(step).toContain('--release-tag "$TAG"')
    expect(step.indexOf('cap-release-changelog.mjs')).toBeLessThan(
      step.indexOf('## Install `browseros-cli`'),
    )
  })

  test('uses the capped notes file for both create and edit reruns', () => {
    const createStep = cliCreateGithubReleaseStep()

    expect(
      createStep.match(/--notes-file \/tmp\/release-notes\.md/g),
    ).toHaveLength(2)
  })
})

function git(cwd: string, args: string[]): string {
  return execFileSync('git', args, {
    cwd,
    encoding: 'utf-8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim()
}
