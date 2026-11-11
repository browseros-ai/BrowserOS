import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { claimStateFolder, liveViewer, portFree } from './running-viewer'

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

test('a state folder belongs to one instance: another refuses it before changing anything', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'neo-viewer-state-'))
  dirs.push(dir)
  await claimStateFolder(dir, 'vp-a')
  await claimStateFolder(dir, 'vp-a')
  await expect(claimStateFolder(dir, 'vp-b')).rejects.toThrow(
    "holds instance vp-a's state",
  )
})

test('two starts claiming one new folder at once: exactly one instance gets it', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'neo-viewer-race-'))
  dirs.push(dir)
  const results = await Promise.allSettled([
    claimStateFolder(dir, 'vp-a'),
    claimStateFolder(dir, 'vp-b'),
  ])
  expect(
    results.filter((result) => result.status === 'fulfilled'),
  ).toHaveLength(1)
  expect(results.filter((result) => result.status === 'rejected')).toHaveLength(
    1,
  )
})

test('a port another server holds is not free; a released one is', () => {
  const holder = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    fetch: () => new Response(null),
  })
  const port = holder.port ?? 0
  expect(portFree(port)).toBe(false)
  holder.stop(true)
  expect(portFree(port)).toBe(true)
})
