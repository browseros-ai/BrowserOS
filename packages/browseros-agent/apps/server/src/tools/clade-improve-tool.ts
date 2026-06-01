import { execFileSync } from 'node:child_process'
import { z } from 'zod'
import { logger } from '../lib/logger'
import { defineToolWithCategory } from './framework'

const defineTriosTool = defineToolWithCategory('trios')

const CLI_PATH =
  '/Users/playra/BrowserOS-full/trios/target/release/clade-improve'

// Only these subcommands may ever be invoked. Capability restriction over
// detection: the CLI is run with execFileSync (no shell), and the arg is
// matched against this allowlist, so no model/tool input can inject flags,
// redirects, or extra commands regardless of what it contains.
const ALLOWED_COMMANDS = ['check', 'constitution', 'rollback'] as const
type CladeCommand = (typeof ALLOWED_COMMANDS)[number]

function runCli(command: CladeCommand): string {
  if (!ALLOWED_COMMANDS.includes(command)) {
    return `EXIT 1: refusing to run disallowed command "${command}"`
  }
  try {
    return execFileSync(CLI_PATH, [command], {
      encoding: 'utf-8',
      env: {
        ...process.env,
        PATH: '/usr/local/bin:/usr/bin:/bin',
      },
      timeout: 30_000,
    })
  } catch (e: any) {
    return `EXIT ${e.status}: ${e.stderr || e.message}`
  }
}

export const cladeImproveStatus = defineTriosTool({
  name: 'cladeImprove_status',
  description:
    'Check which variant (prod/staging/dev) is running, verify safety status',
  parameters: z.object({}),
  execute: async () => {
    logger.info('[cladeImprove] status check')
    return runCli('check')
  },
})

export const cladeImproveConstitution = defineTriosTool({
  name: 'cladeImprove_constitution',
  description: 'Show the Safety Constitution (9 principles)',
  parameters: z.object({}),
  execute: async () => {
    logger.info('[cladeImprove] constitution')
    return runCli('constitution')
  },
})

export const cladeImproveRollback = defineTriosTool({
  name: 'cladeImprove_rollback',
  description:
    'Emergency rollback to previous version. Preserves N=5 versions.',
  parameters: z.object({}),
  execute: async () => {
    logger.warn('[cladeImprove] emergency rollback')
    return runCli('rollback')
  },
})

export default [
  cladeImproveStatus,
  cladeImproveConstitution,
  cladeImproveRollback,
]
