import { afterEach, expect, test } from 'bun:test'
import {
  mkdir,
  mkdtemp,
  readdir,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  claimStateFolder,
  liveViewer,
  otherInstanceStates,
  portFree,
  writeFileAtomically,
} from './running-viewer'

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

test("other instances' states: the folders beside this one and under the state root, never its own", async () => {
  const base = await mkdtemp(join(tmpdir(), 'neo-viewer-states-'))
  dirs.push(base)
  const state = (tag: string) =>
    JSON.stringify({
      lastBuild: { tag, id: `sha256:${tag}`, fingerprint: null },
    })
  const write = async (folder: string, text: string | null) => {
    await mkdir(folder, { recursive: true })
    if (text !== null) await writeFile(join(folder, 'start-state.json'), text)
  }
  await write(join(base, 'scratch', 'mine'), state('mine'))
  await write(join(base, 'scratch', 'beside'), state('beside'))
  await write(join(base, 'scratch', 'never-built'), null)
  await write(join(base, 'scratch', 'torn'), '{"lastBuild":')
  await write(join(base, 'root', 'default'), state('default'))
  const tags = (
    await otherInstanceStates(join(base, 'scratch', 'mine'), join(base, 'root'))
  )
    .map((other) => other.lastBuild?.tag)
    .sort()
  expect(tags).toEqual(['beside', 'default'])
  // An instance under the root sees the others there, and not itself twice.
  const fromRoot = await otherInstanceStates(
    join(base, 'root', 'default'),
    join(base, 'root'),
  )
  expect(fromRoot).toEqual([])
})

test('a state file is replaced whole: a reader sees the old text or the new, and no temporary is left', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'neo-viewer-atomic-'))
  dirs.push(dir)
  const file = join(dir, 'start-state.json')
  await writeFile(file, '{"lastBuild":null}')
  const big = JSON.stringify({
    lastBuild: null,
    kept: Array.from({ length: 5_000 }, (_, i) => ({
      tag: `t${i}`,
      id: `sha256:${i}`,
    })),
  })
  const reads: string[] = []
  const writing = writeFileAtomically(file, big)
  for (let i = 0; i < 20; i++) reads.push(await readFile(file, 'utf8'))
  await writing
  reads.push(await readFile(file, 'utf8'))
  for (const text of reads) expect(['{"lastBuild":null}', big]).toContain(text)
  expect(reads.at(-1)).toBe(big)
  expect(await readdir(dir)).toEqual(['start-state.json'])
})
