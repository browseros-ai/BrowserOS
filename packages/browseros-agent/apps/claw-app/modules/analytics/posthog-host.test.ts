import { describe, expect, it } from 'bun:test'
import { tmpdir } from 'node:os'

/** Exercise build-time env loading in isolation: this module reads its host at import. */
function configuredHost(host?: string): string {
  const env = { ...process.env }
  delete env.VITE_CLAW_POSTHOG_HOST
  if (host !== undefined) env.VITE_CLAW_POSTHOG_HOST = host
  const result = Bun.spawnSync({
    cmd: [
      process.execPath,
      '--eval',
      `import { createPostHogConfig } from ${JSON.stringify(`${import.meta.dir}/posthog.ts`)};
       console.log(JSON.stringify(createPostHogConfig('test-install').api_host));`,
    ],
    // Keep local env files from supplying the optional release variable.
    cwd: tmpdir(),
    env,
  })
  expect(result.exitCode).toBe(0)
  return JSON.parse(result.stdout.toString().trim())
}

describe('PostHog release host', () => {
  it.each([undefined, '', '   '])(
    'sends analytics to the default ingestion host when host is %j',
    (host) => {
      const configured = configuredHost(host)
      expect(configured).toBe('https://us.i.posthog.com')
      for (const path of ['/e/', '/array/test-key/config']) {
        expect(
          new URL(`${configured}${path}`, 'chrome-extension://app/newtab.html')
            .protocol,
        ).toBe('https:')
      }
    },
  )

  it('preserves an explicitly configured ingestion host', () => {
    expect(configuredHost('https://eu.i.posthog.com')).toBe(
      'https://eu.i.posthog.com',
    )
  })
})
