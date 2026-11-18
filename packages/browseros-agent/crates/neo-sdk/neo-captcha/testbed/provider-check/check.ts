import { checkProviders, loadKeys, redact } from './providers'

async function main(): Promise<number> {
  const keys = await loadKeys(process.env)
  const output = (text: string, error = false) => {
    const safe = redact(`${text}\n`, keys)
    if (error) process.stderr.write(safe)
    else process.stdout.write(safe)
  }
  try {
    const args = process.argv.slice(2)
    const unknown = args.find(
      (arg) => !['--balance-only', '--json'].includes(arg),
    )
    if (unknown !== undefined) {
      output(
        `Unknown argument: ${unknown}\nUsage: bun run captcha:provider-check [--balance-only] [--json]`,
        true,
      )
      return 2
    }
    const { report } = await checkProviders(
      { balanceOnly: args.includes('--balance-only') },
      keys,
    )
    if (args.includes('--json')) output(JSON.stringify(report))
    else {
      const lines = [`captcha provider check ${report.timestamp}`]
      for (const vendor of report.vendors) {
        const balance =
          vendor.balance === null ? 'n/a' : `$${vendor.balance.toFixed(2)}`
        let task: string = vendor.task.status
        if (task === 'ok')
          task += ` ${vendor.task.elapsedMs}ms tiles=${JSON.stringify(vendor.task.tiles)}`
        if (task === 'error') task += ` ${vendor.task.errorCode}`
        const message = vendor.message.replace(/[\r\n\t]/g, ' ')
        lines.push(
          `${vendor.vendor.padEnd(12)}${vendor.status.padEnd(8)}balance=${balance.padEnd(6)} task=${task}  ${message}`.trimEnd(),
        )
      }
      lines.push(`paid tasks created: ${report.paidTasksCreated}`)
      const { capsolver, '2captcha': twoCaptcha } = report.paidSolvingReady
      lines.push(
        `paid solving ready: capsolver=${capsolver ? 'yes' : 'no'} 2captcha=${twoCaptcha ? 'yes' : 'no'}`,
      )
      output(lines.join('\n'))
    }
    return report.exitCode
  } catch (error) {
    output(String(error instanceof Error ? error.message : error), true)
    return 1
  }
}

process.exitCode = await main()
