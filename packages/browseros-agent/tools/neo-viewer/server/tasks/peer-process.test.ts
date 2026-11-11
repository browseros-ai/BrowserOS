import { expect, test } from 'bun:test'
import {
  ownerFromNetstat,
  ownerFromSs,
  PeerProcessFinder,
  parsePs,
} from './peer-process'

const NETSTAT = `Active Internet connections (including servers)
Proto Recv-Q Send-Q  Local Address          Foreign Address        (state)          rxbytes      txbytes  rhiwat  shiwat          process:pid    state  options           gencnt    flags   flags1 usecnt rtncnt fltrs
tcp4       0      0  127.0.0.1.7310         127.0.0.1.52344        ESTABLISHED          548          312  408300  146808        bun:41000  00102 00000000 0000000003240e4b 00000081 04000900      1      0 000001
tcp4       0      0  127.0.0.1.52344        127.0.0.1.7310         ESTABLISHED          312          548  408300  146808    2.1.322:24222  00102 00000008 0000000003240e34 00000081 04000900      2      0 000001
tcp4       0      0  127.0.0.1.52345        127.0.0.1.7310         ESTABLISHED          312          548  408300  146808  Code Helper:3131  00102 00000008 0000000003240e35 00000081 04000900      2      0 000001
`

test('netstat: the row whose local end is the agent port names its process', () => {
  expect(
    ownerFromNetstat(NETSTAT, { address: '127.0.0.1', port: 52344 }, 7310),
  ).toBe(24222)
  expect(
    ownerFromNetstat(NETSTAT, { address: '127.0.0.1', port: 52345 }, 7310),
  ).toBe(3131)
  expect(
    ownerFromNetstat(NETSTAT, { address: '127.0.0.1', port: 52399 }, 7310),
  ).toBeNull()
})

test('ss: the row whose local end is the agent port names its process', () => {
  const table = `0      0      127.0.0.1:7310   127.0.0.1:40100 users:(("bun",pid=900,fd=12))
0      0      127.0.0.1:40100  127.0.0.1:7310  users:(("claude",pid=901,fd=30))
`
  expect(ownerFromSs(table, { address: '127.0.0.1', port: 40100 }, 7310)).toBe(
    901,
  )
})

test('ps: start time then command', () => {
  expect(parsePs('Wed Nov  1 10:14:03 2026 /usr/local/bin/claude\n')).toEqual({
    startedAt: 'Wed Nov  1 10:14:03 2026',
    command: '/usr/local/bin/claude',
  })
})

test('one lookup per connection while it is in use; a reused port after idle is looked up again', async () => {
  let clock = 0
  const commands: string[] = []
  const finder = new PeerProcessFinder({
    listenPort: 7310,
    platform: 'darwin',
    now: () => clock,
    reuseMs: 1000,
    run: async (command) => {
      commands.push(command)
      return command === 'netstat' ? NETSTAT : 'Wed Nov 11 10:14:03 2026 claude'
    },
  })
  const peer = { address: '127.0.0.1', port: 52344 }
  const first = finder.find(peer)
  expect(await first.lookup).toEqual({
    found: true,
    pid: 24222,
    startedAt: 'Wed Nov 11 10:14:03 2026',
    command: 'claude',
  })
  const second = finder.find(peer)
  await second.lookup
  first.release()
  second.release()
  expect(commands).toEqual(['netstat', 'ps'])
  clock = 5000
  await finder.find(peer).lookup
  expect(commands.filter((command) => command === 'netstat')).toHaveLength(2)
})

test('a lookup that fails or is too slow is "not found", with the reason', async () => {
  const failing = new PeerProcessFinder({
    listenPort: 7310,
    platform: 'darwin',
    run: async () => {
      throw new Error('denied')
    },
  })
  expect(
    await failing.find({ address: '127.0.0.1', port: 1 }).lookup,
  ).toMatchObject({ found: false })
  const slow = new PeerProcessFinder({
    listenPort: 7310,
    platform: 'darwin',
    timeoutMs: 20,
    run: () => new Promise(() => {}),
  })
  expect(await slow.find({ address: '127.0.0.1', port: 1 }).lookup).toEqual({
    found: false,
    reason: 'process lookup took over 20 ms',
  })
})

test('a reused pid on a new connection is read again, so its new start time makes a new process', async () => {
  let clock = 0
  let starts = 0
  const finder = new PeerProcessFinder({
    listenPort: 7310,
    platform: 'darwin',
    now: () => clock,
    reuseMs: 1000,
    run: async (command) => {
      if (command === 'netstat') return NETSTAT
      starts++
      return `Wed Nov 11 10:14:0${starts} 2026 claude`
    },
  })
  const peer = { address: '127.0.0.1', port: 52344 }
  const first = finder.find(peer)
  const before = await first.lookup
  first.release()
  clock = 5000
  const after = await finder.find(peer).lookup
  expect(before).toMatchObject({
    pid: 24222,
    startedAt: 'Wed Nov 11 10:14:01 2026',
  })
  expect(after).toMatchObject({
    pid: 24222,
    startedAt: 'Wed Nov 11 10:14:02 2026',
  })
})
