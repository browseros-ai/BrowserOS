import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { launchBrowser } from '../../claw-mcp/tests/browser'

/** Preserve argument boundaries without changing the shared browser harness. */
export async function launchConfiguredBrowser(
  headed: boolean,
  args: string[],
  launch = launchBrowser,
) {
  const binary = process.env.BROWSEROS_BINARY
  if (!binary)
    throw new Error('BROWSEROS_BINARY is required unless --mcp-url is set')
  const directory = await mkdtemp(join(tmpdir(), 'captcha-browser-launch-'))
  const previousHeadless = process.env.BROWSEROS_TEST_HEADLESS
  const quote = (value: string) => `'${value.replaceAll("'", "'\"'\"'")}'`
  try {
    const wrapper = join(directory, 'launch')
    await writeFile(
      wrapper,
      `#!/bin/sh\nexec ${quote(binary)} "$@" ${args.map(quote).join(' ')}\n`,
      { mode: 0o700 },
    )
    process.env.BROWSEROS_BINARY = wrapper
    process.env.BROWSEROS_TEST_HEADLESS = headed ? 'false' : 'true'
    return await launch()
  } finally {
    process.env.BROWSEROS_BINARY = binary
    if (previousHeadless === undefined)
      delete process.env.BROWSEROS_TEST_HEADLESS
    else process.env.BROWSEROS_TEST_HEADLESS = previousHeadless
    await rm(directory, { recursive: true, force: true })
  }
}
