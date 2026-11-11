/**
 * Whether this tool's packages are installed, checked before the viewer loads any of them.
 *
 * A fresh checkout has none until `bun install` runs in `packages/browseros-agent`, and only the
 * Bun the repo pins (`packageManager` in its package.json) links this workspace: an older Bun's
 * install reports no changes and links nothing. So with the pinned Bun the command installs them
 * itself; with another it stops with the one command to run.
 */

import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

/** What the viewer's server and page import from outside Bun and Node. */
const NEEDED = [
  'react',
  'react-dom',
  'tailwindcss',
  '@tailwindcss/node',
  '@tailwindcss/oxide',
]

export type DependencyPlan =
  | { kind: 'ready' }
  | { kind: 'install'; command: string[]; cwd: string }
  | { kind: 'stop'; message: string }

/** The packages `toolDir` cannot resolve. */
export function missingDependencies(
  toolDir: string,
  needed: string[] = NEEDED,
): string[] {
  return needed.filter((name) => {
    try {
      Bun.resolveSync(name, toolDir)
      return false
    } catch {
      return true
    }
  })
}

/** `bun@1.4.2` in the agent package's `packageManager` → `1.4.2`. */
export async function pinnedBun(agentDir: string): Promise<string | null> {
  try {
    const manifest = JSON.parse(
      await readFile(join(agentDir, 'package.json'), 'utf8'),
    ) as { packageManager?: string }
    return /^bun@(.+)$/.exec(manifest.packageManager ?? '')?.[1] ?? null
  } catch {
    return null
  }
}

/** Ready, install with the running (pinned) Bun, or stop with the exact command. */
export function planDependencies(input: {
  missing: string[]
  bunVersion: string
  pinned: string | null
  agentDir: string
}): DependencyPlan {
  if (input.missing.length === 0) return { kind: 'ready' }
  if (input.pinned === null || input.bunVersion === input.pinned) {
    return {
      kind: 'install',
      command: ['bun', 'install', '--frozen-lockfile'],
      cwd: input.agentDir,
    }
  }
  return {
    kind: 'stop',
    message: `neo-viewer: its packages (${input.missing.join(', ')}) are not installed, and this is Bun ${input.bunVersion}, whose install does not link them; the repo pins Bun ${input.pinned}. Run: cd ${input.agentDir} && npx -y bun@${input.pinned} install --frozen-lockfile`,
  }
}
