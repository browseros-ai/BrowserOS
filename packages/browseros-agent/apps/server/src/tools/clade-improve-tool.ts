import { execFileSync } from 'node:child_process'
import { z } from 'zod'
import { logger } from '../lib/logger'
import { defineToolWithCategory } from './framework'

const defineTriosTool = defineToolWithCategory('scripts')

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
  } catch (e) {
    const err = e as { status?: number; stderr?: string; message?: string }
    return `EXIT ${err.status}: ${err.stderr || err.message}`
  }
}

export const cladeImproveStatus = defineTriosTool({
  name: 'cladeImprove_status',
  description:
    'Check which variant (prod/staging/dev) is running, verify safety status',
  input: z.object({}),
  handler: async (_args, _ctx, response) => {
    logger.info('cladeImprove status check')
    response.text(runCli('check'))
  },
})

export const cladeImproveConstitution = defineTriosTool({
  name: 'cladeImprove_constitution',
  description: 'Show the Safety Constitution (9 principles)',
  input: z.object({}),
  handler: async (_args, _ctx, response) => {
    logger.info('cladeImprove constitution')
    response.text(runCli('constitution'))
  },
})

export const cladeImproveRollback = defineTriosTool({
  name: 'cladeImprove_rollback',
  description:
    'Emergency rollback to previous version. Preserves N=5 versions.',
  input: z.object({}),
  handler: async (_args, _ctx, response) => {
    logger.warn('cladeImprove emergency rollback')
    response.text(runCli('rollback'))
  },
})

export default [
  cladeImproveStatus,
  cladeImproveConstitution,
  cladeImproveRollback,
]
