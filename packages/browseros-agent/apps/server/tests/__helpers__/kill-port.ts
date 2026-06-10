import { execSync } from 'node:child_process'

export async function killProcessOnPort(port: number): Promise<void> {
  try {
    console.log(`Finding process on port ${port}...`)

    const pids = execSync(`lsof -tiTCP:${port} -sTCP:LISTEN`, {
      encoding: 'utf-8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim()

    if (pids) {
      const pidList = pids.replace(/\n/g, ', ')
      console.log(`Terminating process(es) ${pidList} on port ${port}...`)

      try {
        execSync(`kill -15 ${pids.replace(/\n/g, ' ')}`, {
          stdio: 'ignore',
        })
        await new Promise((resolve) => setTimeout(resolve, 500))
      } catch {
        execSync(`kill -9 ${pids.replace(/\n/g, ' ')}`, {
          stdio: 'ignore',
        })
      }

      console.log(`Terminated process on port ${port}`)
    }
  } catch {
    console.log(`No process found on port ${port}`)
  }

  console.log('Waiting 1 second for port to be released...')
  await new Promise((resolve) => setTimeout(resolve, 1000))
}
