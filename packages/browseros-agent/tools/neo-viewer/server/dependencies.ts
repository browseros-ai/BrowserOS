/**
 * Whether this tool's packages are installed, and current, checked before the viewer loads any of
 * them or starts any container.
 *
 * A fresh checkout has none until `bun install` runs in `packages/browseros-agent`; a checkout
 * pulled after the tool gained a package has it missing; one pulled after a version bump has it
 * stale. Each fails late and cryptically otherwise ("Bundle failed"). The check reads the tool's
 * own package.json for what it needs and `bun.lock` for the versions it must have. Only the Bun
 * the repo pins (`packageManager`) links this workspace: an older Bun's install reports no changes
 * and links nothing. So with the pinned Bun the command installs; with another it stops with the
 * one command to run.
 */

import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

/** A package the tool needs that is missing, or installed at another version than the lockfile's. */
export interface DependencyProblem {
  name: string
  installed: string | null
  locked: string | null
}

export type DependencyPlan =
  | { kind: 'ready' }
  | { kind: 'install'; command: string[]; cwd: string }
  | { kind: 'stop'; message: string }

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

/** `bun@1.4.2` in the agent package's `packageManager` → `1.4.2`. */
export async function pinnedBun(agentDir: string): Promise<string | null> {
  const manifest = (await readJson(join(agentDir, 'package.json'))) as {
    packageManager?: string
  } | null
  return /^bun@(.+)$/.exec(manifest?.packageManager ?? '')?.[1] ?? null
}

/** One line when the running Bun is not the pinned one, whose install is the only one that links this tool. */
export function bunNote(
  bunVersion: string,
  pinned: string | null,
): string | null {
  if (pinned === null || bunVersion === pinned) return null
  return `Note: this is Bun ${bunVersion}; the repo pins Bun ${pinned}, and only its install links this tool's packages.`
}

/** Ready, install with the running (pinned) Bun, or stop with the exact command. */
export function planDependencies(input: {
  problems: DependencyProblem[]
  bunVersion: string
  pinned: string | null
  agentDir: string
}): DependencyPlan {
  if (input.problems.length === 0) return { kind: 'ready' }
  if (input.pinned === null || input.bunVersion === input.pinned) {
    return {
      kind: 'install',
      command: ['bun', 'install', '--frozen-lockfile'],
      cwd: input.agentDir,
    }
  }
  return {
    kind: 'stop',
    message: `neo-viewer: its packages are not installed or older than bun.lock (${input.problems.map(describeProblem).join(', ')}), and this is Bun ${input.bunVersion}, whose install does not link them; the repo pins Bun ${input.pinned}. Run: cd ${shellQuote(input.agentDir)} && npx -y bun@${input.pinned} install --frozen-lockfile`,
  }
}

/** A path as one shell word, so the printed command can be pasted as is. */
function shellQuote(path: string): string {
  return /^[\w./@:+-]+$/.test(path)
    ? path
    : `'${path.replaceAll("'", "'\\''")}'`
}
