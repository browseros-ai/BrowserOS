import { expect, test } from 'bun:test'
import { DEMO_PAGES, FIXTURE_PAGES } from './catalog'

test('tests that the CLI prints public fixture and demo URLs and stops on SIGINT', async () => {
  const child = Bun.spawn(
    [
      'bun',
      `${import.meta.dir}/serve.ts`,
      '--host',
      '0.0.0.0',
      '--public-host',
      'captcha.test',
      '--port',
      '0',
    ],
    { stdout: 'pipe', stderr: 'pipe' },
  )
  try {
    const reader = child.stdout.getReader()
    let output = ''
    while (!output.includes('Ctrl-C')) {
      const { value, done } = await reader.read()
      if (done) break
      output += new TextDecoder().decode(value)
    }
    expect(output).toContain('http://captcha.test:')
    for (const page of FIXTURE_PAGES)
      expect(output).toContain(page.path ?? 'missing-path')
    for (const page of DEMO_PAGES)
      expect(output).toContain(page.url ?? 'missing-url')
    child.kill('SIGINT')
    expect(await child.exited).toBe(0)
  } finally {
    child.kill()
  }
})

test('tests that the CLI rejects unknown arguments and invalid ports', async () => {
  for (const [args, message] of [
    [['--wat'], 'Unknown option'],
    [['--port', 'bad'], 'Invalid port'],
    [['--port=-1'], 'Invalid port'],
    [['--host'], 'argument missing'],
  ] as const) {
    const child = Bun.spawn(['bun', `${import.meta.dir}/serve.ts`, ...args], {
      stdout: 'ignore',
      stderr: 'pipe',
    })
    expect(await child.exited).not.toBe(0)
    expect(await new Response(child.stderr).text()).toContain(message)
  }
})
