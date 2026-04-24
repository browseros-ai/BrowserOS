import { spawnSync } from 'node:child_process'
import { existsSync, mkdirSync, readdirSync } from 'node:fs'
import { dirname, resolve } from 'node:path'

const projectRoot = resolve(import.meta.dir, '..', '..')
const testsRoot = resolve(projectRoot, 'tests')
const cleanupScript = resolve(testsRoot, '__helpers__/cleanup.sh')
const testPreloadPath = './tests/__helpers__/test-env.ts'
const preferredDirectoryGroups = [
  'agent',
  'api',
  'skills',
  'tools',
  'browser',
  'sdk',
]
const ignoredDirectories = new Set(['__fixtures__', '__helpers__'])
const rootGroupExclusions = new Set(['server.integration.test.ts'])
const testFilePattern = /\.(test|spec)\.[cm]?[jt]sx?$/

function compareGroupNames(left: string, right: string): number {
  const leftIndex = preferredDirectoryGroups.indexOf(left)
  const rightIndex = preferredDirectoryGroups.indexOf(right)
  const leftRank =
    leftIndex === -1 ? preferredDirectoryGroups.length : leftIndex
  const rightRank =
    rightIndex === -1 ? preferredDirectoryGroups.length : rightIndex
  if (leftRank !== rightRank) {
    return leftRank - rightRank
  }
  return left.localeCompare(right)
}

function listDirectoryGroups(): string[] {
  return readdirSync(testsRoot, { withFileTypes: true })
    .filter(
      (entry) => entry.isDirectory() && !ignoredDirectories.has(entry.name),
    )
    .map((entry) => entry.name)
    .sort(compareGroupNames)
}

function listRootTestTargets(): string[] {
  return readdirSync(testsRoot, { withFileTypes: true })
    .filter((entry) => entry.isFile() && testFilePattern.test(entry.name))
    .filter((entry) => !rootGroupExclusions.has(entry.name))
    .map((entry) => `./tests/${entry.name}`)
    .sort((left, right) => left.localeCompare(right))
}

function listAllGroups(): string[] {
  const groups = [...listDirectoryGroups()]
  if (existsSync(resolve(testsRoot, 'server.integration.test.ts'))) {
    groups.push('integration')
  }
  if (listRootTestTargets().length > 0) {
    groups.push('root')
  }
  return groups
}

function listAvailableGroupNames(): string[] {
  return ['all', 'core', 'cdp', ...listAllGroups()].sort((left, right) =>
    left.localeCompare(right),
  )
}

function getCompositeGroupMembers(group: string): string[] | null {
  if (group === 'all') {
    return listAllGroups()
  }
  if (group === 'core') {
    return ['agent', 'api', 'skills', 'root']
  }
  return null
}

function getAtomicGroupTargets(group: string): string[] {
  if (group === 'cdp') {
    return getAtomicGroupTargets('browser')
  }
  if (group === 'integration') {
    return existsSync(resolve(testsRoot, 'server.integration.test.ts'))
      ? ['./tests/server.integration.test.ts']
      : []
  }
  if (group === 'root') {
    return listRootTestTargets()
  }
  if (existsSync(resolve(testsRoot, group))) {
    return [`./tests/${group}`]
  }
  return []
}

function runCommand(cmd: string[], label: string): number {
  console.log(`\n==> ${label}`)
  const result = spawnSync(cmd[0], cmd.slice(1), {
    cwd: projectRoot,
    env: withTestEnv(process.env),
    stdio: 'inherit',
  })

  if (result.error) {
    throw result.error
  }

  return result.status ?? 1
}

export function withTestEnv(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  if (env.NODE_ENV) return env
  return { ...env, NODE_ENV: 'test' }
}

export function buildTestCommand(
  targets: string[],
  junitPath?: string,
): string[] {
  const cmd = [
    process.execPath,
    '--env-file=.env.development',
    'test',
    `--preload=${testPreloadPath}`,
  ]
  if (junitPath) {
    const outputPath = resolve(projectRoot, junitPath)
    mkdirSync(dirname(outputPath), { recursive: true })
    cmd.push('--reporter=junit', `--reporter-outfile=${outputPath}`)
  }
  cmd.push(...targets)
  return cmd
}

function runAtomicGroup(group: string): number {
  const targets = getAtomicGroupTargets(group)
  if (targets.length === 0) {
    throw new Error(
      `Unknown test group "${group}". Available groups: ${listAvailableGroupNames().join(', ')}`,
    )
  }
  runCommand(['bash', cleanupScript], `Cleaning up test resources for ${group}`)
  const junitPath = process.env.BROWSEROS_JUNIT_PATH?.trim()
  const cmd = buildTestCommand(targets, junitPath)
  return runCommand(cmd, `Running ${group} tests`)
}

function runGroup(group: string): number {
  const compositeMembers = getCompositeGroupMembers(group)
  if (compositeMembers) {
    let exitCode = 0
    for (const member of compositeMembers) {
      const status = runGroup(member)
      if (status !== 0 && exitCode === 0) {
        exitCode = status
      }
    }
    return exitCode
  }
  return runAtomicGroup(group)
}

if (import.meta.main) {
  const requestedGroup = process.argv[2] ?? 'all'
  process.exit(runGroup(requestedGroup))
}
