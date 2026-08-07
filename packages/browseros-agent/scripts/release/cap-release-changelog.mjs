#!/usr/bin/env node

import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'

export const DEFAULT_MAX_ENTRIES = 15

/** Builds the GitHub compare URL used when a release changelog is truncated. */
export function buildCompareUrl(options) {
  const previousTag = options.previousTag ?? ''
  if (!previousTag) {
    throw new Error('Cannot build a compare URL without a previous tag')
  }

  const releaseTag = options.releaseTag ?? ''
  if (!releaseTag) {
    throw new Error('Cannot build a compare URL without a release tag')
  }

  const repositoryUrl = resolveRepositoryUrl(options)
  if (!repositoryUrl) {
    throw new Error('Cannot build a compare URL without a repository URL')
  }

  return `${repositoryUrl}/compare/${encodeURIComponent(previousTag)}...${encodeURIComponent(releaseTag)}`
}

/** Caps a changelog-only Markdown fragment while leaving non-entry prose intact. */
export function renderCappedChangelog(content, options = {}) {
  const maxEntries = options.maxEntries ?? DEFAULT_MAX_ENTRIES
  assertMaxEntries(maxEntries)

  const lines = content.replace(/\r\n/g, '\n').split('\n')
  if (lines.at(-1) === '') {
    lines.pop()
  }

  let entryCount = 0
  let lastKeptEntryIndex = -1
  const output = []

  for (const line of lines) {
    if (isChangelogEntry(line)) {
      entryCount += 1
      if (entryCount > maxEntries) {
        continue
      }
      lastKeptEntryIndex = output.length
    }

    output.push(line)
  }

  if (entryCount <= maxEntries) {
    return content
  }

  const compareUrl = buildCompareUrl(options)
  output.splice(
    lastKeptEntryIndex + 1,
    0,
    '',
    `_Showing the latest ${maxEntries} of ${entryCount} changes. [View the full changelog](${compareUrl})._`,
  )

  return `${output.join('\n')}\n`
}

function isChangelogEntry(line) {
  return line.startsWith('- ')
}

function assertMaxEntries(maxEntries) {
  if (!Number.isInteger(maxEntries) || maxEntries < 1) {
    throw new Error(`Expected --max-entries to be a positive integer`)
  }
}

function resolveRepositoryUrl(options) {
  if (options.repositoryUrl) {
    return normalizeRepositoryUrl(options.repositoryUrl)
  }

  const serverUrl = options.githubServerUrl ?? process.env.GITHUB_SERVER_URL
  const repository = options.githubRepository ?? process.env.GITHUB_REPOSITORY
  if (serverUrl && repository) {
    return normalizeRepositoryUrl(
      `${serverUrl.replace(/\/+$/, '')}/${repository.replace(/^\/+|\/+$/g, '')}`,
    )
  }

  return detectOriginRepositoryUrl(options.repositoryRoot ?? process.cwd())
}

function detectOriginRepositoryUrl(repositoryRoot) {
  try {
    return normalizeRepositoryUrl(
      execFileSync('git', ['config', '--get', 'remote.origin.url'], {
        cwd: repositoryRoot,
        encoding: 'utf-8',
        stdio: ['ignore', 'pipe', 'ignore'],
      }).trim(),
    )
  } catch {
    return ''
  }
}

function normalizeRepositoryUrl(repositoryUrl) {
  const trimmed = repositoryUrl
    .trim()
    .replace(/\/+$/, '')
    .replace(/\.git$/, '')
  const scpLike = trimmed.match(/^git@([^:]+):(.+)$/)
  if (scpLike) {
    return `https://${scpLike[1]}/${scpLike[2].replace(/\.git$/, '')}`
  }

  const ssh = trimmed.match(/^ssh:\/\/git@([^/]+)\/(.+)$/)
  if (ssh) {
    return `https://${ssh[1]}/${ssh[2].replace(/\.git$/, '')}`
  }

  return trimmed
}

function parseArgs(args) {
  const options = {}
  for (let index = 0; index < args.length; index += 1) {
    const raw = args[index]
    if (!raw.startsWith('--')) {
      throw new Error(`Unexpected argument ${raw}`)
    }

    const key = raw.slice(2)
    const value = args[index + 1]
    if (value === undefined) {
      throw new Error(`Missing value for --${key}`)
    }

    options[key] = value
    index += 1
  }
  return options
}

function requireOption(options, key) {
  const value = options[key]
  if (value === undefined || value === '') {
    throw new Error(`Missing --${key}`)
  }
  return value
}

function toCliOptions(args) {
  const parsed = parseArgs(args)
  const maxEntries = parsed['max-entries']
    ? Number(parsed['max-entries'])
    : DEFAULT_MAX_ENTRIES

  return {
    input: requireOption(parsed, 'input'),
    output: requireOption(parsed, 'output'),
    maxEntries,
    previousTag: parsed['previous-tag'] ?? '',
    releaseTag: parsed['release-tag'] ?? '',
    repositoryUrl: parsed['repository-url'] ?? '',
    githubServerUrl: parsed['github-server-url'] ?? '',
    githubRepository: parsed['github-repository'] ?? '',
    repositoryRoot: parsed['repository-root'] ?? process.cwd(),
  }
}

function main() {
  const options = toCliOptions(process.argv.slice(2))
  const content = readFileSync(options.input, 'utf-8')
  const rendered = renderCappedChangelog(content, options)
  writeFileSync(options.output, rendered)
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  try {
    main()
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    console.error(`::error::${message}`)
    process.exit(1)
  }
}
