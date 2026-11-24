import { describe, expect, test } from 'bun:test'
import type { FirefoxLogin } from '../src/firefox/logins'
import {
  type PasswordSyncState,
  planPasswordSync,
  type SavedCredential,
  signonRealmFor,
} from '../src/neo/passwords'

function login(overrides: Partial<FirefoxLogin>): FirefoxLogin {
  return {
    guid: '{1}',
    origin: 'https://example.com',
    httpRealm: null,
    username: 'alice',
    password: 'pw',
    timePasswordChanged: 100,
    ...overrides,
  }
}

function saved(realm: string, username: string): SavedCredential {
  return { id: 7, username, signonRealms: [realm], entry: { id: 7, username } }
}

const kinds = (actions: ReturnType<typeof planPasswordSync>) =>
  actions.map((a) => (a.kind === 'skip' ? `skip:${a.reason}` : a.kind))

describe('signonRealmFor', () => {
  test('is the origin plus a slash, punycoded', () => {
    expect(signonRealmFor('https://example.com')).toBe('https://example.com/')
    expect(signonRealmFor('http://example.com:8080')).toBe(
      'http://example.com:8080/',
    )
    expect(signonRealmFor('https://пример.рф')).toBe(
      'https://xn--e1afmkfd.xn--p1ai/',
    )
  })

  test('rejects non-web origins', () => {
    expect(signonRealmFor('chrome://FirefoxAccounts')).toBeUndefined()
    expect(signonRealmFor('android://hash@com.example/')).toBeUndefined()
    expect(signonRealmFor('not a url')).toBeUndefined()
  })
})

describe('planPasswordSync', () => {
  test('first sync adds new logins and lets Firefox win over existing ones', () => {
    const actions = planPasswordSync(
      [login({}), login({ guid: '{2}', username: 'bob' })],
      [saved('https://example.com/', 'bob')],
      {},
    )
    expect(kinds(actions)).toEqual(['add', 'update'])
  })

  test('repeat sync leaves unchanged logins alone and updates changed ones', () => {
    const state: PasswordSyncState = {
      '{1}': { timePasswordChanged: 100 },
      '{2}': { timePasswordChanged: 100 },
    }
    const actions = planPasswordSync(
      [
        login({}),
        login({ guid: '{2}', username: 'bob', timePasswordChanged: 200 }),
      ],
      [
        saved('https://example.com/', 'alice'),
        saved('https://example.com/', 'bob'),
      ],
      state,
    )
    expect(kinds(actions)).toEqual(['update', 'unchanged'])
  })

  test('respects a deletion in neo until Firefox changes the password', () => {
    const state: PasswordSyncState = { '{1}': { timePasswordChanged: 100 } }
    expect(kinds(planPasswordSync([login({})], [], state))).toEqual([
      'skip:removed-in-neo',
    ])
    expect(
      kinds(planPasswordSync([login({ timePasswordChanged: 300 })], [], state)),
    ).toEqual(['add'])
  })

  test('skips what the password manager cannot hold as a form login', () => {
    const actions = planPasswordSync(
      [
        login({ guid: '{h}', httpRealm: 'Staff' }),
        login({ guid: '{c}', origin: 'chrome://FirefoxAccounts' }),
        login({ guid: '{e}', username: 'x', password: '' }),
      ],
      [],
      {},
    )
    expect(kinds(actions)).toEqual([
      'skip:http-auth',
      'skip:unsupported-origin',
      'skip:empty-password',
    ])
  })

  test('keeps the newest of duplicate origin+username logins', () => {
    const actions = planPasswordSync(
      [
        login({ guid: '{old}', timePasswordChanged: 1 }),
        login({ guid: '{new}', timePasswordChanged: 2 }),
      ],
      [],
      {},
    )
    expect(actions.map((a) => [a.kind, a.login.guid])).toEqual([
      ['add', '{new}'],
      ['skip', '{old}'],
    ])
  })
})
