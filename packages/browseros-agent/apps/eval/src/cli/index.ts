import { startDashboard } from '../dashboard/server'
import { runEval } from '../runs/eval-runner'
import { type EvalCliArgs, parseEvalCliArgs } from './args'
import { runGradeCommand } from './commands/grade'
import { publishRun, runPublishCommand } from './commands/publish'
import { runRunCommand } from './commands/run'
import { runSuiteCommand } from './commands/suite'

function usage(): string {
  return `
BrowserOS Eval

Usage:
  bun run eval suite --config <config.json> [--publish r2]
  bun run eval suite --suite <suite.json> --variant <id> [--publish r2]
  bun run eval run --config <config.json>
  bun run eval run --suite <suite.json> --variant <id>
  bun run eval grade --run <results/run-dir>
  bun run eval publish --run <results/run-dir> --target r2
  bun run eval -c <config.json>
`
}

async function runLegacyCommand(args: EvalCliArgs): Promise<void> {
  if (args.command !== 'legacy') return
  if (args.help) {
    console.log(usage())
    return
  }
  if (args.configPath) {
    await runEval({ configPath: args.configPath })
    return
  }

  startDashboard({
    tasks: [],
    configName: '',
    agentType: '',
    outputDir: '',
    configMode: true,
  })
  console.log(
    'Dashboard running at http://localhost:9900 — configure and run from the UI',
  )
  await new Promise(() => {})
}

/** Dispatches the eval CLI while preserving the old config/dashboard entry points. */
export async function runCli(
  argv: string[] = Bun.argv.slice(2),
): Promise<void> {
  const args = parseEvalCliArgs(argv)
  switch (args.command) {
    case 'legacy':
      await runLegacyCommand(args)
      break
    case 'suite':
      await runSuiteCommand(args, { publishRun })
      break
    case 'run':
      await runRunCommand(args)
      break
    case 'grade':
      await runGradeCommand(args)
      break
    case 'publish':
      await runPublishCommand(args)
      break
  }
}
