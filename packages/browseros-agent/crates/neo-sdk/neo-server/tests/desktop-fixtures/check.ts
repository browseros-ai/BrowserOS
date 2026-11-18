/** CI verifies offline guards and runner types before entering the source-built browser lifecycle. */
import { resolve } from 'node:path'
import { validateInventory } from './inventory'

const AGENT_ROOT = resolve(import.meta.dir, '../../../../..')

async function execute(args: string[]): Promise<void> {
  const child = Bun.spawn({
    cmd: [process.execPath, ...args],
    cwd: AGENT_ROOT,
    stdin: 'ignore',
    stdout: 'inherit',
    stderr: 'inherit',
  })
  const signals: NodeJS.Signals[] = ['SIGINT', 'SIGTERM', 'SIGHUP']
  const handlers = signals.map((signal) => {
    const handler = () => child.kill(signal)
    process.on(signal, handler)
    return { signal, handler }
  })
  try {
    const code = await child.exited
    if (code !== 0)
      throw new Error(`Desktop fixture check command exited ${code}`)
  } finally {
    for (const { signal, handler } of handlers) process.off(signal, handler)
  }
}

if (import.meta.main) {
  try {
    const args = process.argv.slice(2)
    if (args.length > 0 && args[0] !== '--run')
      throw new Error('Usage: bun check.ts [--run <run.ts options>]')
    await validateInventory()
    await execute([
      'test',
      import.meta.dir,
      resolve(AGENT_ROOT, 'ci/affected-suites.test.ts'),
    ])
    await execute([
      resolve(AGENT_ROOT, 'node_modules/typescript/bin/tsc'),
      '--project',
      resolve(import.meta.dir, 'tsconfig.json'),
    ])
    if (args[0] === '--run')
      await execute([resolve(import.meta.dir, 'run.ts'), ...args.slice(1)])
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error))
    process.exitCode = 1
  }
}
