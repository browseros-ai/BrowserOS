/** `bun run neo:viewer`: starts the viewer and prints where to point the agent and the browser. */

import { configFrom } from './config'
import { startViewer } from './viewer-server'

const config = configFrom(process.argv.slice(2), process.env)
const viewer = await startViewer(config)

console.log(`neo viewer: ${viewer.url}
  agent MCP URL: ${viewer.mcpUrl}  (forwards to ${config.upstream.href})
  task logs:     ${config.dataDir}/tasks`)

let stopping = false
for (const signal of ['SIGINT', 'SIGTERM'] as const) {
  process.on(signal, async () => {
    if (stopping) return
    stopping = true
    await viewer.stop()
    process.exit(0)
  })
}
