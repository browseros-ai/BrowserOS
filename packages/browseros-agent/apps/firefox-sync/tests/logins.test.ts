import { Database } from 'bun:sqlite'
import { afterEach, describe, expect, test } from 'bun:test'
import { rmSync } from 'node:fs'
import { join } from 'node:path'
import { readFirefoxLogins } from '../src/firefox/logins'
import { PrimaryPasswordError } from '../src/firefox/nss'
import {
  type FixtureLogin,
  octets,
  oid,
  seq,
  tempProfile,
  writeLoginStore,
} from './fixtures'

const LOGINS: FixtureLogin[] = [
  {
    guid: '{a}',
    hostname: 'https://example.com',
    username: 'alice@example.com',
    password: 'p@ss wörd',
    timePasswordChanged: 1_700_000_000_000,
  },
  {
    guid: '{b}',
    hostname: 'https://intranet.example',
    httpRealm: 'Staff',
    username: 'bob',
    password: 'hunter2',
    timePasswordChanged: 1_700_000_100_000,
  },
]

const dirs: string[] = []
function profile() {
  const dir = tempProfile()
  dirs.push(dir)
  return dir
}
afterEach(() => {
  for (const dir of dirs.splice(0))
    rmSync(dir, { recursive: true, force: true })
})

describe('readFirefoxLogins', () => {
  test('decrypts a current (PBES2 + AES-256) profile without a Primary Password', () => {
    const dir = profile()
    writeLoginStore(dir, LOGINS)
    const { logins, undecryptable } = readFirefoxLogins(dir)
    expect(undecryptable).toBe(0)
    expect(logins).toEqual([
      {
        guid: '{a}',
        origin: 'https://example.com',
        httpRealm: null,
        username: 'alice@example.com',
        password: 'p@ss wörd',
        timePasswordChanged: 1_700_000_000_000,
      },
      {
        guid: '{b}',
        origin: 'https://intranet.example',
        httpRealm: 'Staff',
        username: 'bob',
        password: 'hunter2',
        timePasswordChanged: 1_700_000_100_000,
      },
    ])
  })

  test('decrypts an old (SHA1 + 3DES) profile', () => {
    const dir = profile()
    writeLoginStore(dir, LOGINS, { scheme: '3des' })
    expect(readFirefoxLogins(dir).logins.map((l) => l.password)).toEqual([
      'p@ss wörd',
      'hunter2',
    ])
  })

  test('picks the AES key when a migrated profile also keeps the old 3DES key', () => {
    const dir = profile()
    writeLoginStore(dir, LOGINS, { legacyKeyFirst: true })
    const { logins, undecryptable } = readFirefoxLogins(dir)
    expect(undecryptable).toBe(0)
    expect(logins.map((l) => l.username)).toEqual(['alice@example.com', 'bob'])
  })

  test('asks for the Primary Password when one is set', () => {
    const dir = profile()
    writeLoginStore(dir, LOGINS, { primaryPassword: 'secret' })
    expect(() => readFirefoxLogins(dir)).toThrow(PrimaryPasswordError)
    expect(() => readFirefoxLogins(dir, 'wrong')).toThrow(
      'Primary Password is wrong',
    )
    expect(readFirefoxLogins(dir, 'secret').logins).toHaveLength(2)
  })

  test('a key4.db it cannot parse is not reported as a wrong Primary Password', () => {
    const dir = profile()
    writeLoginStore(dir, LOGINS)
    const db = new Database(join(dir, 'key4.db'))
    db.query("UPDATE metaData SET item2 = ? WHERE id = 'password'").run(
      seq(seq(oid('1.2.3.4'), seq()), octets(Buffer.alloc(16))),
    )
    db.close()
    let error: unknown
    try {
      readFirefoxLogins(dir)
    } catch (e) {
      error = e
    }
    expect(error).not.toBeInstanceOf(PrimaryPasswordError)
    expect((error as Error).message).toContain(
      'Unsupported key4.db algorithm 1.2.3.4',
    )
  })

  test('a profile without logins.json yields nothing', () => {
    expect(readFirefoxLogins(profile())).toEqual({
      logins: [],
      undecryptable: 0,
    })
  })
})
