import { parseArgs } from 'node:util'
import { DEMO_PAGES, FIXTURE_PAGES } from './catalog'
import { startCaptchaFixtureServer } from './server'

try {
  const { values } = parseArgs({
    args: process.argv.slice(2),
    strict: true,
    options: {
      host: { type: 'string' },
      'public-host': { type: 'string' },
      port: { type: 'string' },
    },
  })
  const port = values.port === undefined ? undefined : Number(values.port)
  if (
    port !== undefined &&
    (!Number.isInteger(port) ||
      port < 0 ||
      port > 65535 ||
      !values.port?.trim())
  )
    throw new Error('Invalid port: expected 0–65535')
  const server = await startCaptchaFixtureServer({
    hostname: values.host,
    publicHost: values['public-host'],
    port,
  })
  for (const page of FIXTURE_PAGES)
    console.log(`${page.id}: ${server.url(page.path ?? '/')}`)
  for (const page of DEMO_PAGES) console.log(`${page.id}: ${page.url}`)
  console.log('Press Ctrl-C to stop.')
  const stop = async () => {
    await server.stop()
    process.exit(0)
  }
  process.once('SIGINT', stop)
  process.once('SIGTERM', stop)
} catch (error) {
  console.error(error instanceof Error ? error.message : error)
  process.exit(1)
}
