/**
 * Whether this tool's packages are installed, and current, checked before the viewer loads any of
 * them or starts any container.
 *
 * A fresh checkout has none until `bun install` runs in `packages/browseros-agent`; a checkout
 * pulled after the tool gained a package has it missing; one pulled after a version bump has it
 * stale. Each fails late and cryptically otherwise ("Bundle failed"). The check reads the tool's
 * own package.json for what it needs and `bun.lock` for the versions it must have. When something
 * is missing, the running Bun installs it (`--frozen-lockfile`, so bun.lock is never rewritten)
 * and the check runs again; only a Bun too old to install this workspace, or an install that
 * still left something missing, stops the start with the one command to run.
 */

import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

/** A package the tool needs that is missing, or installed at another version than the lockfile's. */
export interface DependencyProblem {
  name: string
  installed: string | null
  locked: string | null
}

const WORKSPACE = '@browseros/neo-viewer'

/** The tool's packages (type-only `@types/*` aside) whose install is missing or not the lockfile's. */
export async function dependencyProblems(
  toolDir: string,
  agentDir: string,
): Promise<DependencyProblem[]> {
  const manifest = (await readJson(join(toolDir, 'package.json'))) as {
    dependencies?: Record<string, string>
    devDependencies?: Record<string, string>
  } | null
  const names = Object.keys({
    ...manifest?.dependencies,
    ...manifest?.devDependencies,
  }).filter((name) => !name.startsWith('@types/'))
  const locked = await lockedVersions(agentDir)
  const problems: DependencyProblem[] = []
  for (const name of names) {
    const installed =
      (
        (await readJson(
          join(toolDir, 'node_modules', name, 'package.json'),
        )) as { version?: string } | null
      )?.version ?? null
    const want = locked.get(`${WORKSPACE}/${name}`) ?? locked.get(name) ?? null
    if (installed === null || (want !== null && installed !== want))
      problems.push({ name, installed, locked: want })
  }
  return problems
}

/** `name 19.2.0 (lockfile 19.3.0)`, or `name missing`. */
export function describeProblem(problem: DependencyProblem): string {
  if (problem.installed === null) return `${problem.name} missing`
  return `${problem.name} ${problem.installed} (lockfile ${problem.locked})`
}

/**
 * `bun.lock`'s resolved versions by package key. The file is JSON with trailing commas; each
 * package entry starts with `"name@version"`.
 */
async function lockedVersions(agentDir: string): Promise<Map<string, string>> {
  const versions = new Map<string, string>()
  let text: string
  try {
    text = await readFile(join(agentDir, 'bun.lock'), 'utf8')
  } catch {
    return versions
  }
  const lock = JSON.parse(text.replace(/,(\s*[}\]])/g, '$1')) as {
    packages?: Record<string, unknown[]>
  }
  for (const [key, entry] of Object.entries(lock.packages ?? {})) {
    const spec = entry[0]
    if (typeof spec !== 'string') continue
    const at = spec.lastIndexOf('@')
    if (at > 0) versions.set(key, spec.slice(at + 1))
  }
  return versions
}

async function readJson(path: string): Promise<unknown> {
  try {
    return JSON.parse(await readFile(path, 'utf8'))
  } catch {
    return null
  }
}

/**
 * The oldest Bun that installs this workspace's packages for the tool and runs the viewer.
 * Proved on 2026-11-11 (macOS arm64, this workspace: `linker = "isolated"`, lockfile version 1):
 * Bun 1.2.19, 1.3.0, 1.3.6, 1.4.2 and 1.4.4 each linked a package missing from the tool, a package
 * a pull added to it, a stale link and a fresh checkout with `bun install --frozen-lockfile`, left
 * bun.lock untouched, then built the page and started the viewer. Bun 1.2.18 does not know
 * `linker = "isolated"` (it arrived in 1.2.19): it installs hoisted and the tool gets no
 * node_modules. Every version prints "(no changes)" even while it links, which is not a failure.
 * The tool's own test suite needs Bun 1.4: older bundlers fail to read acorn.mjs again when the
 * whole suite runs in one process (each file passes alone, and the viewer builds its page once).
 */
export const MIN_INSTALL_BUN = '1.2.19'

/** `bun@1.4.2` in the agent package's `packageManager` → `1.4.2`: a Bun to suggest. */
export async function pinnedBun(agentDir: string): Promise<string | null> {
  const manifest = (await readJson(join(agentDir, 'package.json'))) as {
    packageManager?: string
  } | null
  return /^bun@(.+)$/.exec(manifest?.packageManager ?? '')?.[1] ?? null
}

export type DependencyPlan =
  | { kind: 'ready' }
  | { kind: 'install'; args: string[]; cwd: string; note: string }
  | { kind: 'stop'; message: string }

interface PlanInput {
  problems: DependencyProblem[]
  bunVersion: string
  /** The repo's `packageManager` Bun, suggested in a stop's command; the minimum when unknown. */
  pinned: string | null
  agentDir: string
}

/** Ready; install with the running Bun; or, for a Bun too old to install, stop with the command. */
export function planDependencies(input: PlanInput): DependencyPlan {
  if (input.problems.length === 0) return { kind: 'ready' }
  const list = input.problems.map(describeProblem).join(', ')
  if (Bun.semver.order(input.bunVersion, MIN_INSTALL_BUN) < 0)
    return {
      kind: 'stop',
      message: `neo-viewer: its packages are missing or older than bun.lock (${list}), and Bun ${input.bunVersion} cannot install them: it needs Bun ${MIN_INSTALL_BUN} or newer. Run: ${installCommand(input)}`,
    }
  return {
    kind: 'install',
    args: ['install', '--frozen-lockfile'],
    cwd: input.agentDir,
    note: `Installing the viewer's packages with Bun ${input.bunVersion} (${list}): bun install --frozen-lockfile in ${input.agentDir}`,
  }
}

/** After the install: ready only when nothing is missing any more, checked, never assumed. */
export function afterInstall(
  input: PlanInput & { exitCode: number },
): { kind: 'ready' } | { kind: 'stop'; message: string } {
  if (input.exitCode !== 0)
    return {
      kind: 'stop',
      message: `neo-viewer: bun install --frozen-lockfile with Bun ${input.bunVersion} failed (exit ${input.exitCode}; its output is above). Run: ${installCommand(input)}`,
    }
  if (input.problems.length > 0)
    return {
      kind: 'stop',
      message: `neo-viewer: bun install --frozen-lockfile with Bun ${input.bunVersion} did not link ${input.problems.map(describeProblem).join(', ')}. Run: ${installCommand(input)}`,
    }
  return { kind: 'ready' }
}

/** What a start does about the packages: nothing to do, installed (re-run the viewer), or stop. */
export type DependencyOutcome =
  | { kind: 'ready' }
  | { kind: 'installed' }
  | { kind: 'stop'; message: string; output?: string }

/**
 * The whole check: look, install with the running Bun if needed (saying so in one line), look
 * again. `install` runs `bun <args>` in `cwd` and hands back its exit code and output, which is
 * shown only when it fails.
 */
export async function ensureDependencies(input: {
  toolDir: string
  agentDir: string
  bunVersion: string
  install: (
    args: string[],
    cwd: string,
  ) => Promise<{ exitCode: number; output: string }>
  say: (line: string) => void
}): Promise<DependencyOutcome> {
  const base = {
    bunVersion: input.bunVersion,
    pinned: await pinnedBun(input.agentDir),
    agentDir: input.agentDir,
  }
  const plan = planDependencies({
    ...base,
    problems: await dependencyProblems(input.toolDir, input.agentDir),
  })
  if (plan.kind !== 'install') return plan
  input.say(plan.note)
  const { exitCode, output } = await input.install(plan.args, plan.cwd)
  const after = afterInstall({
    ...base,
    exitCode,
    problems:
      exitCode === 0
        ? await dependencyProblems(input.toolDir, input.agentDir)
        : [],
  })
  if (after.kind === 'stop') return { ...after, output }
  return { kind: 'installed' }
}

function installCommand(input: {
  pinned: string | null
  agentDir: string
}): string {
  return `cd ${shellQuote(input.agentDir)} && npx -y bun@${input.pinned ?? MIN_INSTALL_BUN} install --frozen-lockfile`
}

/** A path as one shell word, so the printed command can be pasted as is. */
function shellQuote(path: string): string {
  return /^[\w./@:+-]+$/.test(path)
    ? path
    : `'${path.replaceAll("'", "'\\''")}'`
}
