import { fixtureRouteList, startBrowserFixtureServer } from './app'

const port = process.env.BROWSEROS_FIXTURE_PORT
  ? Number(process.env.BROWSEROS_FIXTURE_PORT)
  : undefined

const server = await startBrowserFixtureServer({ port })

console.log(`BrowserOS tool fixtures: ${server.baseUrl}`)
for (const route of fixtureRouteList) {
  console.log(`- ${route.title}: ${server.url(route.path)}`)
}
console.log('Press Ctrl+C to stop.')

let stopping = false
async function stop(): Promise<void> {
  if (stopping) return
  stopping = true
  await server.stop()
  process.exit(0)
}

process.on('SIGINT', () => {
  stop().catch((err) => {
    console.error(err)
    process.exit(1)
  })
})
process.on('SIGTERM', () => {
  stop().catch((err) => {
    console.error(err)
    process.exit(1)
  })
})

await new Promise(() => {})
