import { describe, expect, it } from 'bun:test'

import {
  buildCompareUrl,
  renderCappedChangelog,
} from './cap-release-changelog.mjs'

const repositoryUrl = 'https://github.test/browseros-ai/BrowserOS'
const previousTag = 'claw-server/v0.0.5'
const releaseTag = 'claw-server/v0.0.6'

function changelogEntries(count: number): string {
  return Array.from({ length: count }, (_, index) => `- change ${index + 1}`)
    .join('\n')
    .concat('\n')
}

function render(content: string): string {
  return renderCappedChangelog(content, {
    maxEntries: 15,
    previousTag,
    releaseTag,
    repositoryUrl,
  })
}

function changelistEntries(content: string): string[] {
  return content.split('\n').filter((line) => line.startsWith('- '))
}

describe('renderCappedChangelog', () => {
  it('preserves empty changelogs', () => {
    expect(render('No notable changes.\n')).toBe('No notable changes.\n')
  })

  it('preserves a single changelog entry without a compare link', () => {
    expect(render(changelogEntries(1))).toBe('- change 1\n')
  })

  it('preserves exactly fifteen changelog entries without a compare link', () => {
    const content = changelogEntries(15)

    expect(render(content)).toBe(content)
  })

  it('keeps the newest fifteen entries and appends a compare link', () => {
    const result = render(changelogEntries(17))

    expect(changelistEntries(result)).toHaveLength(15)
    expect(result).toContain('- change 1')
    expect(result).toContain('- change 15')
    expect(result).not.toContain('- change 16')
    expect(result).not.toContain('- change 17')
    expect(result).toContain(
      '_Showing the latest 15 of 17 changes. [View the full changelog](https://github.test/browseros-ai/BrowserOS/compare/claw-server%2Fv0.0.5...claw-server%2Fv0.0.6)._',
    )
  })

  it('places the truncation notice after the retained changelist', () => {
    const result = render(`## What's Changed\n\n${changelogEntries(16)}`)

    expect(result.indexOf('- change 15')).toBeLessThan(
      result.indexOf('_Showing the latest 15 of 16 changes.'),
    )
    expect(
      result.indexOf('_Showing the latest 15 of 16 changes.'),
    ).toBeLessThan(result.length)
  })

  it('preserves initial release text when no previous tag exists', () => {
    const content = 'Initial BrowserClaw Server release.\n'

    expect(
      renderCappedChangelog(content, {
        maxEntries: 15,
        previousTag: '',
        releaseTag,
        repositoryUrl,
      }),
    ).toBe(content)
  })

  it('does not fabricate a compare URL when truncation needs a previous tag', () => {
    expect(() =>
      renderCappedChangelog(changelogEntries(16), {
        maxEntries: 15,
        previousTag: '',
        releaseTag,
        repositoryUrl,
      }),
    ).toThrow('previous tag')
  })
})

describe('buildCompareUrl', () => {
  it('encodes slash-containing release tags', () => {
    expect(
      buildCompareUrl({
        previousTag: 'cli/v0.4.0',
        releaseTag: 'cli/v0.4.1',
        repositoryUrl: 'https://github.com/browseros-ai/BrowserOS.git',
      }),
    ).toBe(
      'https://github.com/browseros-ai/BrowserOS/compare/cli%2Fv0.4.0...cli%2Fv0.4.1',
    )
  })

  it('can build from GitHub Actions server and repository values', () => {
    expect(
      buildCompareUrl({
        previousTag,
        releaseTag,
        githubServerUrl: 'https://github.enterprise.test/',
        githubRepository: '/browseros-ai/BrowserOS/',
      }),
    ).toBe(
      'https://github.enterprise.test/browseros-ai/BrowserOS/compare/claw-server%2Fv0.0.5...claw-server%2Fv0.0.6',
    )
  })
})
