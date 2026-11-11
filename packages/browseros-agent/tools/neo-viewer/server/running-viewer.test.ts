import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { liveViewer } from './running-viewer'

const dirs: string[] = []
afterEach(async () => {
  for (const dir of dirs.splice(0))
    await rm(dir, { recursive: true, force: true })
})

async function recorded(record: object): Promise<string> {
  const dir = await mkdtemp(join(tmpdir(), 'neo-viewer-running-'))
  dirs.push(dir)
  const file = join(dir, 'viewer.json')
  await writeFile(file, JSON.stringify(record))
  return file
}

test("a recorded viewer counts only while its pid still runs this tool's entry", async () => {
  const file = await recorded({
    pid: 4242,
    port: 7310,
    url: 'http://127.0.0.1:7310',
  })
  expect(
    await liveViewer(file, async () => 'bun server/main.ts --name default'),
  ).toMatchObject({ pid: 4242, port: 7310 })
  // The viewer crashed and the OS gave its pid to something else, or it is gone.
  expect(await liveViewer(file, async () => '/usr/bin/vim notes.md')).toBeNull()
  expect(await liveViewer(file, async () => null)).toBeNull()
  expect(await liveViewer(join(tmpdir(), 'no-such-viewer.json'))).toBeNull()
})
