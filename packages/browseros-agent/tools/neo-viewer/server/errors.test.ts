import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { describeError } from './errors'

const dirs: string[] = []
afterEach(async () => {
  for (const dir of dirs.splice(0))
    await rm(dir, { recursive: true, force: true })
})

test('a failed page build says which file imports which missing module, not only "Bundle failed"', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'neo-viewer-build-'))
  dirs.push(dir)
  await writeFile(
    join(dir, 'index.html'),
    '<script type="module" src="./main.tsx"></script>',
  )
  await writeFile(
    join(dir, 'main.tsx'),
    "import '@fontsource/not-installed/latin-400.css'\nexport {}\n",
  )
  // As server/page.ts builds: throw: false keeps the bundler's messages.
  const result = await Bun.build({
    entrypoints: [join(dir, 'index.html')],
    target: 'browser',
    throw: false,
  })
  expect(result.success).toBe(false)
  const text = describeError(
    new AggregateError(result.logs, 'neo-viewer: the web page did not build'),
  )
  expect(text).toStartWith('neo-viewer: the web page did not build\n  ')
  expect(text).toContain('main.tsx:1')
  expect(text).toContain('@fontsource/not-installed/latin-400.css')
})

test('a plain error is its message; a non-error is its text', () => {
  expect(describeError(new Error('docker run failed: no space'))).toBe(
    'docker run failed: no space',
  )
  expect(describeError('gone')).toBe('gone')
})
