/**
 * `bun run neo:viewer` starts here. It imports only Bun and Node built-ins: the viewer's own
 * modules import packages (React, Tailwind) a fresh checkout may not have yet, and a missing one
 * would fail at load, before anything could say what to do; a stale one fails later, in the page
 * build. So it checks them against the tool's package.json and bun.lock first (`dependencies.ts`),
 * installs or explains, then loads the viewer (`launch.ts`).
 */

import { join } from 'node:path'
import {
  bunNote,
  dependencyProblems,
  describeProblem,
  pinnedBun,
  planDependencies,
} from './dependencies'

const TOOL_DIR = join(import.meta.dir, '..')
const AGENT_DIR = join(TOOL_DIR, '..', '..')

const pinned = await pinnedBun(AGENT_DIR)
const note = bunNote(Bun.version, pinned)
if (note) console.log(note)
const plan = planDependencies({
  problems: await dependencyProblems(TOOL_DIR, AGENT_DIR),
  bunVersion: Bun.version,
  pinned,
  agentDir: AGENT_DIR,
})
if (plan.kind === 'stop') {
  console.error(plan.message)
  process.exit(1)
}
if (plan.kind === 'install') {
  console.log(
    `Installing the viewer's packages: ${plan.command.join(' ')} (in ${plan.cwd})`,
  )
  // The running binary is the pinned Bun; `bun` on PATH may be another.
  const install = Bun.spawn([process.execPath, ...plan.command.slice(1)], {
    cwd: plan.cwd,
    stdout: 'inherit',
    stderr: 'inherit',
  })
  const missing =
    (await install.exited) === 0
      ? (await dependencyProblems(TOOL_DIR, AGENT_DIR)).map(describeProblem)
      : ['(install failed)']
  if (missing.length > 0) {
    console.error(
      `neo-viewer: still missing ${missing.join(', ')}; run ${plan.command.join(' ')} in ${plan.cwd}`,
    )
    process.exit(1)
  }
  // This process looked the packages up before they existed and Bun keeps that answer, so the
  // viewer runs in a fresh one. Signals sent to this process are passed on (Ctrl-C reaches both
  // through the terminal's process group; the viewer ignores the copy).
  const viewer = Bun.spawn([process.execPath, ...process.argv.slice(1)], {
    stdin: 'inherit',
    stdout: 'inherit',
    stderr: 'inherit',
  })
  process.on('SIGINT', () => viewer.kill('SIGINT'))
  process.on('SIGTERM', () => viewer.kill('SIGTERM'))
  process.exit(await viewer.exited)
}
await import('./launch')
