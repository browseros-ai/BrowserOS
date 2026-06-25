import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'

type TestCommand = {
  label: string
  cwd?: string
  argv: readonly [string, ...string[]]
}

const projectRoot = resolve(import.meta.dir, '..')
const bun = process.execPath

const testSuites = {
  all: [
    {
      label: 'server tests',
      cwd: resolve(projectRoot, 'apps/server'),
      argv: [bun, 'run', 'test'],
    },
    {
      label: 'claw-app tests',
      cwd: resolve(projectRoot, 'apps/claw-app'),
      argv: [bun, 'run', 'test'],
    },
    {
      label: 'claw-server tests',
      cwd: resolve(projectRoot, 'apps/claw-server'),
      argv: [bun, 'run', 'test'],
    },
    {
      label: 'agent tests',
      cwd: resolve(projectRoot, 'apps/app'),
      argv: [bun, 'run', 'test'],
    },
    {
      label: 'eval tests',
      cwd: resolve(projectRoot, 'apps/eval'),
      argv: [bun, 'run', 'test'],
    },
    {
      label: 'build script tests',
      argv: [bun, 'run', './scripts/run-bun-test.ts', './scripts/build'],
    },
    {
      label: 'release script tests',
      argv: [bun, 'run', './scripts/run-bun-test.ts', './scripts/release'],
    },
  ],
  main: [
    {
      label: 'server tools tests',
      cwd: resolve(projectRoot, 'apps/server'),
      argv: [bun, 'run', 'test:tools'],
    },
    {
      label: 'server integration tests',
      cwd: resolve(projectRoot, 'apps/server'),
      argv: [bun, 'run', 'test:integration'],
    },
  ],
} satisfies Record<string, readonly TestCommand[]>

type TestSuiteName = keyof typeof testSuites

function isTestSuiteName(value: string): value is TestSuiteName {
  return value in testSuites
}

/** Prevents multi-step suites from overwriting a single shared JUnit report path. */
function buildCommandEnv(): NodeJS.ProcessEnv {
  const env = { ...process.env }
  delete env.BROWSEROS_JUNIT_PATH
  return env
}

function runCommand(command: TestCommand): number {
  console.log(`\n==> ${command.label}`)
  const result = spawnSync(command.argv[0], command.argv.slice(1), {
    cwd: command.cwd ?? projectRoot,
    env: buildCommandEnv(),
    stdio: 'inherit',
  })
  if (result.error) {
    throw result.error
  }
  if (result.signal) {
    console.error(
      `Command terminated by signal ${result.signal}: ${command.label}`,
    )
    return 1
  }
  const status = result.status ?? 1
  if (status !== 0) {
    console.error(`Command failed with exit code ${status}: ${command.label}`)
  }
  return status
}

/** Runs a named test suite without shell chaining so each step reports its own status. */
function runSuite(suiteName: TestSuiteName): number {
  let exitCode = 0
  for (const command of testSuites[suiteName]) {
    const status = runCommand(command)
    if (status !== 0 && exitCode === 0) {
      exitCode = status
    }
  }
  return exitCode
}

function printUsage(): void {
  console.error(
    `Usage: bun run ./scripts/run-test-suite.ts <${Object.keys(testSuites).join('|')}>`,
  )
}

if (import.meta.main) {
  const requestedSuite = process.argv[2]
  if (!requestedSuite || !isTestSuiteName(requestedSuite)) {
    printUsage()
    process.exit(1)
  }
  process.exit(runSuite(requestedSuite))
}
