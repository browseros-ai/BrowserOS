/**
 * `bun run neo:viewer` starts here. It imports only Bun and Node built-ins: the viewer's own
 * modules import packages (React, Tailwind) a fresh checkout may not have yet, and a missing one
 * would fail at load, before anything could say what to do; a stale one fails later, in the page
 * build. So it checks them against the tool's package.json and bun.lock first (`dependencies.ts`),
 * installs them with the running Bun when needed (or explains, for a Bun too old to), then loads
 * the viewer (`launch.ts`).
 */

import { join } from 'node:path'
import { ensureDependencies } from './dependencies'

const TOOL_DIR = join(import.meta.dir, '..')
const AGENT_DIR = join(TOOL_DIR, '..', '..')

const outcome = await ensureDependencies({
  toolDir: TOOL_DIR,
  agentDir: AGENT_DIR,
  bunVersion: Bun.version,
  // The running Bun installs: `bun` on PATH may be another one.
  install: async (args, cwd) => {
    const install = Bun.spawn([process.execPath, ...args], {
      cwd,
      stdout: 'pipe',
      stderr: 'pipe',
    })
    const [out, err, exitCode] = await Promise.all([
      new Response(install.stdout).text(),
      new Response(install.stderr).text(),
      install.exited,
    ])
    return { exitCode, output: out + err }
  },
  say: (line) => console.log(line),
})
if (outcome.kind === 'stop') {
  if (outcome.output) console.error(outcome.output.trimEnd())
  console.error(outcome.message)
  process.exit(1)
}
if (outcome.kind === 'installed') {
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
