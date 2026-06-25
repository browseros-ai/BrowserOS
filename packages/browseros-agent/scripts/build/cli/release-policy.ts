#!/usr/bin/env bun

import { execFileSync } from 'node:child_process'
import { appendFileSync } from 'node:fs'

const DEFAULT_LATEST_VERSION_URL =
  'https://cdn.browseros.com/cli/latest/version.txt'
const DEFAULT_LATEST_MANIFEST_URL =
  'https://cdn.browseros.com/cli/latest/manifest.json'
const STRICT_VERSION_SOURCE =
  '(?<version>(?:0|[1-9]\\d*)\\.(?:0|[1-9]\\d*)\\.(?:0|[1-9]\\d*))'
const CLI_TAG_PATTERN = new RegExp(`^cli/v${STRICT_VERSION_SOURCE}$`)
const LEGACY_CLI_TAG_PATTERN = new RegExp(
  `^browseros-cli-v${STRICT_VERSION_SOURCE}$`,
)
const VERSION_PATTERN =
  /^(?<major>0|[1-9]\d*)\.(?<minor>0|[1-9]\d*)\.(?<patch>0|[1-9]\d*)$/

export interface ParsedCliReleaseTag {
  tag: string
  version: string
}

export interface CliReleaseValidation {
  tag: string
  version: string
  latestVersion: string
  latestTag: string
  previousTag: string
  targetCommit: string
}

interface ValidateOptions {
  tag: string
  defaultBranch: string
  latestVersionURL: string
  latestManifestURL: string
  repositoryRoot: string
}

type VersionTuple = [number, number, number]

/** Parses the pushed CLI release tag into the version used by build and publish steps. */
export function parseCliReleaseTag(tag: string): ParsedCliReleaseTag {
  const match = tag.match(CLI_TAG_PATTERN)
  const version = match?.groups?.version
  if (!version) {
    throw new Error(
      `Expected pushed tag to match cli/vX.Y.Z, received ${JSON.stringify(tag)}`,
    )
  }
  return { tag, version }
}

/** Compares strict X.Y.Z release versions without accepting prerelease or loose semver forms. */
export function compareReleaseVersions(a: string, b: string): number {
  const left = parseReleaseVersion(a)
  const right = parseReleaseVersion(b)

  for (let index = 0; index < left.length; index += 1) {
    const diff = left[index] - right[index]
    if (diff !== 0) {
      return diff < 0 ? -1 : 1
    }
  }

  return 0
}

/** Fails releases that do not move production latest forward, except same-tag repair reruns. */
export function assertIncrementingRelease(
  version: string,
  latestVersion: string,
  options: { tag?: string; latestTag?: string } = {},
): void {
  const comparison = compareReleaseVersions(version, latestVersion)
  if (comparison > 0) {
    return
  }
  if (
    comparison === 0 &&
    options.tag !== undefined &&
    options.tag === options.latestTag
  ) {
    return
  }

  throw new Error(
    `Release version ${version} must be greater than latest published CLI ${latestVersion}`,
  )
}

/** Picks the closest earlier CLI tag across current and legacy CLI release tag names. */
export function selectPreviousCliReleaseTag(
  tags: string[],
  currentVersion: string,
): string {
  const current = parseReleaseVersion(currentVersion)
  const candidates = tags
    .map(parseKnownCliTag)
    .filter((tag): tag is ParsedCliReleaseTag => tag !== null)
    .filter(
      ({ version }) => compareReleaseVersions(version, currentVersion) < 0,
    )
    .sort((a, b) => {
      const versionOrder = compareReleaseVersions(b.version, a.version)
      if (versionOrder !== 0) {
        return versionOrder
      }
      return tagPriority(b.tag) - tagPriority(a.tag)
    })

  if (candidates.length === 0) {
    return ''
  }

  const selected = candidates[0]
  if (
    compareVersionTuple(parseReleaseVersion(selected.version), current) >= 0
  ) {
    throw new Error(
      `Previous release tag ${selected.tag} is not before ${currentVersion}`,
    )
  }
  return selected.tag
}

/** Runs the release gate used by the GitHub Actions workflow before publishing starts. */
export async function validateCliRelease(
  options: ValidateOptions,
): Promise<CliReleaseValidation> {
  const parsed = parseCliReleaseTag(options.tag)
  const targetCommit = ensureTagReachableFromDefaultBranch(
    options.repositoryRoot,
    parsed.tag,
    options.defaultBranch,
  )
  ensureAnnotatedTag(options.repositoryRoot, parsed.tag)
  const latestVersion = await fetchLatestVersion(options.latestVersionURL)
  const latestTag =
    compareReleaseVersions(parsed.version, latestVersion) === 0
      ? await fetchLatestManifestTag(options.latestManifestURL)
      : ''
  assertIncrementingRelease(parsed.version, latestVersion, {
    tag: parsed.tag,
    latestTag,
  })

  const previousTag = selectPreviousCliReleaseTag(
    listGitTags(options.repositoryRoot),
    parsed.version,
  )

  return {
    tag: parsed.tag,
    version: parsed.version,
    latestVersion,
    latestTag,
    previousTag,
    targetCommit,
  }
}

function parseReleaseVersion(version: string): VersionTuple {
  const match = version.match(VERSION_PATTERN)
  if (!match?.groups) {
    throw new Error(
      `Expected strict release version X.Y.Z, received ${JSON.stringify(version)}`,
    )
  }
  return [
    Number(match.groups.major),
    Number(match.groups.minor),
    Number(match.groups.patch),
  ]
}

function compareVersionTuple(a: VersionTuple, b: VersionTuple): number {
  for (let index = 0; index < a.length; index += 1) {
    const diff = a[index] - b[index]
    if (diff !== 0) {
      return diff < 0 ? -1 : 1
    }
  }
  return 0
}

function parseKnownCliTag(tag: string): ParsedCliReleaseTag | null {
  const match = tag.match(CLI_TAG_PATTERN) ?? tag.match(LEGACY_CLI_TAG_PATTERN)
  const version = match?.groups?.version
  if (!version) {
    return null
  }
  try {
    parseReleaseVersion(version)
  } catch {
    return null
  }
  return { tag, version }
}

/** Enforces annotated release tags so release metadata has an intentional tag object. */
export function ensureAnnotatedTag(repositoryRoot: string, tag: string): void {
  const tagType = runGit(repositoryRoot, ['cat-file', '-t', `refs/tags/${tag}`])
  if (tagType !== 'tag') {
    throw new Error(`Tag ${tag} must be an annotated tag`)
  }
}

function tagPriority(tag: string): number {
  return tag.startsWith('cli/') ? 1 : 0
}

/** Verifies the pushed tag resolves to a commit already contained in the default branch. */
export function ensureTagReachableFromDefaultBranch(
  repositoryRoot: string,
  tag: string,
  defaultBranch: string,
): string {
  runGit(repositoryRoot, [
    'fetch',
    'origin',
    `refs/heads/${defaultBranch}:refs/remotes/origin/${defaultBranch}`,
    '--tags',
  ])

  const targetCommit = runGit(repositoryRoot, ['rev-parse', `${tag}^{commit}`])
  try {
    runGit(repositoryRoot, [
      'merge-base',
      '--is-ancestor',
      targetCommit,
      `origin/${defaultBranch}`,
    ])
  } catch {
    throw new Error(
      `Tag ${tag} targets ${targetCommit}, which is not reachable from origin/${defaultBranch}`,
    )
  }
  return targetCommit
}

function listGitTags(repositoryRoot: string): string[] {
  return runGit(repositoryRoot, ['tag', '-l'])
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean)
}

function runGit(repositoryRoot: string, args: string[]): string {
  return execFileSync('git', args, {
    cwd: repositoryRoot,
    encoding: 'utf-8',
    stdio: ['ignore', 'pipe', 'inherit'],
  }).trim()
}

async function fetchLatestVersion(url: string): Promise<string> {
  const response = await fetch(url)
  if (!response.ok) {
    throw new Error(
      `Failed to fetch latest CLI version from ${url}: HTTP ${response.status}`,
    )
  }

  const version = (await response.text()).trim()
  parseReleaseVersion(version)
  return version
}

async function fetchLatestManifestTag(url: string): Promise<string> {
  const response = await fetch(url)
  if (!response.ok) {
    throw new Error(
      `Failed to fetch latest CLI manifest from ${url}: HTTP ${response.status}`,
    )
  }

  const manifest = (await response.json()) as { tag?: unknown }
  return typeof manifest.tag === 'string' ? manifest.tag : ''
}

function parseArgs(args: string[]): Record<string, string> {
  const options: Record<string, string> = {}
  for (let index = 0; index < args.length; index += 1) {
    const raw = args[index]
    if (!raw.startsWith('--')) {
      throw new Error(`Unexpected argument ${raw}`)
    }
    const key = raw.slice(2)
    const value = args[index + 1]
    if (!value || value.startsWith('--')) {
      throw new Error(`Missing value for --${key}`)
    }
    options[key] = value
    index += 1
  }
  return options
}

function writeGithubOutputs(validation: CliReleaseValidation): void {
  const outputPath = process.env.GITHUB_OUTPUT
  if (!outputPath) {
    return
  }

  appendFileSync(
    outputPath,
    `${[
      `tag=${validation.tag}`,
      `version=${validation.version}`,
      `latest_version=${validation.latestVersion}`,
      `latest_tag=${validation.latestTag}`,
      `previous_tag=${validation.previousTag}`,
      `target_commit=${validation.targetCommit}`,
    ].join('\n')}\n`,
  )
}

async function main(): Promise<void> {
  const [command, ...args] = process.argv.slice(2)
  if (command !== 'validate') {
    throw new Error(
      'Usage: release-policy.ts validate --tag <tag> --default-branch <branch>',
    )
  }

  const options = parseArgs(args)
  const tag = options.tag ?? process.env.RELEASE_TAG
  if (!tag) {
    throw new Error('Missing release tag')
  }

  const defaultBranch =
    options['default-branch'] ?? process.env.DEFAULT_BRANCH ?? 'main'
  const latestVersionURL =
    options['latest-version-url'] ?? DEFAULT_LATEST_VERSION_URL
  const latestManifestURL =
    options['latest-manifest-url'] ?? DEFAULT_LATEST_MANIFEST_URL
  const repositoryRoot = options['repository-root'] ?? process.cwd()

  const validation = await validateCliRelease({
    tag,
    defaultBranch,
    latestVersionURL,
    latestManifestURL,
    repositoryRoot,
  })

  writeGithubOutputs(validation)
  console.log(
    `Validated ${validation.tag}: version ${validation.version}, latest ${validation.latestVersion}, previous ${validation.previousTag || 'none'}`,
  )
}

if (import.meta.main) {
  main().catch((error) => {
    const message = error instanceof Error ? error.message : String(error)
    console.error(`::error::${message}`)
    process.exit(1)
  })
}
