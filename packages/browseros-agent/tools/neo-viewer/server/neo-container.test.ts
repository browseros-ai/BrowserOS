import { describe, expect, test } from 'bun:test'
import { launchOptionsFrom } from './config'
import {
  type CommandResult,
  imageTag,
  NAMES,
  planImage,
  type Runner,
  removeLeftovers,
  removeReplacedImage,
} from './neo-container'

const ok = (stdout = ''): CommandResult => ({ code: 0, stdout, stderr: '' })
const missing: CommandResult = { code: 1, stdout: '', stderr: 'No such object' }

/** A fake `docker` that answers from a table and records every call. */
function fakeDocker(answer: (args: string[]) => CommandResult) {
  const calls: string[][] = []
  const run: Runner = async (command, args) => {
    calls.push([command, ...args])
    return answer(args)
  }
  return { run, calls }
}

describe('the image for this checkout', () => {
  test("is build.sh's tag: browser, 9-character commit, -dirty with changes, arch", () => {
    const parts = {
      browserVersion: '0.52.1.2',
      commit: 'd6b008abb633ed4d',
      arch: 'arm64',
    }
    expect(imageTag({ ...parts, dirty: false })).toBe(
      'neo-sdk:0.52.1.2-d6b008abb-arm64',
    )
    expect(imageTag({ ...parts, dirty: true })).toBe(
      'neo-sdk:0.52.1.2-d6b008abb-dirty-arm64',
    )
  })

  test('a clean checkout reuses its image when it is on the machine, and builds it when not', () => {
    const wanted = { tag: 'neo-sdk:x-arm64', fingerprint: null }
    expect(planImage(wanted, 'sha256:a', { lastBuild: null })).toEqual({
      kind: 'reuse',
      tag: wanted.tag,
    })
    expect(planImage(wanted, null, { lastBuild: null })).toMatchObject({
      kind: 'build',
    })
  })

  test('a changed checkout rebuilds unless the image is the one built from the same changes', () => {
    const wanted = { tag: 'neo-sdk:x-dirty-arm64', fingerprint: 'f1' }
    const built = { tag: wanted.tag, id: 'sha256:a', fingerprint: 'f1' }
    expect(planImage(wanted, 'sha256:a', { lastBuild: built })).toEqual({
      kind: 'reuse',
      tag: wanted.tag,
    })
    // New changes since that build.
    expect(
      planImage({ ...wanted, fingerprint: 'f2' }, 'sha256:a', {
        lastBuild: built,
      }),
    ).toMatchObject({ kind: 'build' })
    // Someone else built this -dirty tag: its contents are unknown.
    expect(planImage(wanted, 'sha256:b', { lastBuild: built })).toMatchObject({
      kind: 'build',
    })
    expect(planImage(wanted, 'sha256:a', { lastBuild: null })).toMatchObject({
      kind: 'build',
    })
  })
})

describe('cleanup touches only what the viewer created', () => {
  test("removes the last start's labelled container, network and volume", async () => {
    const docker = fakeDocker((args) =>
      args[1] === 'inspect' ? ok('1\n') : ok(),
    )
    const removed = await removeLeftovers(docker.run, false)
    expect(removed).toEqual([
      `container ${NAMES.container}`,
      `network ${NAMES.network}`,
      `volume ${NAMES.volume}`,
    ])
    expect(docker.calls.filter((call) => call.includes('rm'))).toEqual([
      ['docker', 'rm', '-f', NAMES.container],
      ['docker', 'network', 'rm', NAMES.network],
      ['docker', 'volume', 'rm', NAMES.volume],
    ])
  })

  test('keeps the volume with --keep-data, and skips names that are not there', async () => {
    const docker = fakeDocker((args) =>
      args[0] === 'network' && args[1] === 'inspect'
        ? missing
        : args[1] === 'inspect'
          ? ok('1')
          : ok(),
    )
    expect(await removeLeftovers(docker.run, true)).toEqual([
      `container ${NAMES.container}`,
    ])
    expect(docker.calls.some((call) => call.includes('volume'))).toBe(false)
  })

  test('stops rather than remove a same-named thing without its label', async () => {
    const docker = fakeDocker((args) =>
      args[1] === 'inspect' ? ok('<no value>') : ok(),
    )
    await expect(removeLeftovers(docker.run, false)).rejects.toThrow(
      'the viewer did not create it',
    )
    expect(docker.calls.some((call) => call.includes('rm'))).toBe(false)
  })

  test('removes only the image its own last build recorded, once a new build replaced it', async () => {
    const docker = fakeDocker(() => ok())
    const previous = {
      tag: 'neo-sdk:old-arm64',
      id: 'sha256:0123456789abcdef',
      fingerprint: null,
    }
    expect(await removeReplacedImage(docker.run, null, 'sha256:new')).toBeNull()
    expect(
      await removeReplacedImage(docker.run, previous, previous.id),
    ).toBeNull()
    expect(
      await removeReplacedImage(docker.run, previous, 'sha256:new'),
    ).toContain('neo-sdk:old-arm64')
    expect(docker.calls).toEqual([['docker', 'image', 'rm', previous.id]])
  })
})

test('launch options: no arguments starts a container; --upstream starts none', () => {
  expect(launchOptionsFrom([], {})).toMatchObject({
    upstream: null,
    keepData: false,
    build: true,
    image: null,
    port: 7310,
  })
  expect(
    launchOptionsFrom(
      ['--keep-data', '--no-build', '--image', 'neo-sdk:t'],
      {},
    ),
  ).toMatchObject({
    keepData: true,
    build: false,
    image: 'neo-sdk:t',
  })
  expect(
    launchOptionsFrom(['--upstream', 'http://127.0.0.1:19010/mcp'], {}).upstream
      ?.href,
  ).toBe('http://127.0.0.1:19010/mcp')
})
