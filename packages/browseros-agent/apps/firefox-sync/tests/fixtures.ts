/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Builds synthetic Firefox profiles for tests: a key4.db and logins.json
 * encrypted the way NSS writes them, a cookies.sqlite with the schema-17
 * table, and mozLz4 session files. No real profile is ever read by tests.
 */

import { Database } from 'bun:sqlite'
import {
  createCipheriv,
  createHash,
  createHmac,
  pbkdf2Sync,
  randomBytes,
} from 'node:crypto'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { OID } from '../src/firefox/nss'

function length(n: number): Buffer {
  if (n < 0x80) return Buffer.from([n])
  const bytes: number[] = []
  for (let v = n; v > 0; v = Math.floor(v / 256)) bytes.unshift(v & 0xff)
  return Buffer.from([0x80 | bytes.length, ...bytes])
}

export function der(tag: number, ...contents: Uint8Array[]): Buffer {
  const body = Buffer.concat(contents)
  return Buffer.concat([Buffer.from([tag]), length(body.length), body])
}

export const seq = (...c: Uint8Array[]) => der(0x30, ...c)
export const octets = (b: Uint8Array) => der(0x04, b)
export const int = (n: number) => {
  const bytes: number[] = []
  for (let v = n; v > 0; v = Math.floor(v / 256)) bytes.unshift(v & 0xff)
  if (bytes.length === 0 || bytes[0] & 0x80) bytes.unshift(0)
  return der(0x02, Buffer.from(bytes))
}
export const nil = () => der(0x05)
export function oid(dotted: string): Buffer {
  const [a, b, ...rest] = dotted.split('.').map(Number)
  const bytes = [a * 40 + b]
  for (const part of rest) {
    const chunk = [part & 0x7f]
    for (let v = Math.floor(part / 128); v > 0; v = Math.floor(v / 128)) {
      chunk.unshift((v & 0x7f) | 0x80)
    }
    bytes.push(...chunk)
  }
  return der(0x06, Buffer.from(bytes))
}

function encrypt(
  algorithm: 'aes-256-cbc' | 'des-ede3-cbc',
  key: Uint8Array,
  iv: Uint8Array,
  data: Uint8Array,
) {
  const c = createCipheriv(algorithm, key, iv)
  return Buffer.concat([c.update(data), c.final()])
}

const sha1 = (...parts: Uint8Array[]) => {
  const h = createHash('sha1')
  for (const p of parts) h.update(p)
  return h.digest()
}

/** A PBES2 blob as Firefox 72+ writes into key4.db (14-byte IV quirk included). */
export function pbes2Blob(globalSalt: Buffer, password: string, plain: Buffer) {
  const salt = randomBytes(32)
  const iv14 = randomBytes(14)
  const iterations = 1000
  const key = pbkdf2Sync(
    sha1(globalSalt, Buffer.from(password)),
    salt,
    iterations,
    32,
    'sha256',
  )
  const iv = Buffer.concat([Buffer.from([0x04, 0x0e]), iv14])
  return seq(
    seq(
      oid(OID.PBES2),
      seq(
        seq(
          oid(OID.PBKDF2),
          seq(
            octets(salt),
            int(iterations),
            int(32),
            seq(oid(OID.HMAC_SHA256), nil()),
          ),
        ),
        seq(oid(OID.AES256_CBC), octets(iv14)),
      ),
    ),
    octets(encrypt('aes-256-cbc', key, iv, plain)),
  )
}

/** A pre-Firefox-72 pbeWithSha1AndTripleDES-CBC blob. */
export function tripleDesBlob(
  globalSalt: Buffer,
  password: string,
  plain: Buffer,
) {
  const entrySalt = randomBytes(20)
  const hp = sha1(globalSalt, Buffer.from(password))
  const chp = sha1(hp, entrySalt)
  const pes = Buffer.alloc(20)
  entrySalt.copy(pes)
  const hmac = (d: Buffer) => createHmac('sha1', chp).update(d).digest()
  const k = Buffer.concat([
    hmac(Buffer.concat([pes, entrySalt])),
    hmac(Buffer.concat([hmac(pes), entrySalt])),
  ])
  return seq(
    seq(oid(OID.PBE_SHA1_3DES), seq(octets(entrySalt), int(1))),
    octets(encrypt('des-ede3-cbc', k.subarray(0, 24), k.subarray(-8), plain)),
  )
}

export function loginField(
  key: Buffer,
  value: string,
  cipher: 'aes' | '3des' = 'aes',
) {
  const iv = randomBytes(cipher === 'aes' ? 16 : 8)
  const data =
    cipher === 'aes'
      ? encrypt('aes-256-cbc', key.subarray(0, 32), iv, Buffer.from(value))
      : encrypt('des-ede3-cbc', key.subarray(0, 24), iv, Buffer.from(value))
  return seq(
    octets(Buffer.from('f8000000000000000000000000000001', 'hex')),
    seq(oid(cipher === 'aes' ? OID.AES256_CBC : OID.DES_EDE3_CBC), octets(iv)),
    octets(data),
  ).toString('base64')
}

export function tempProfile(): string {
  return mkdtempSync(join(tmpdir(), 'firefox-sync-test-'))
}

export interface FixtureLogin {
  guid: string
  hostname: string
  httpRealm?: string | null
  username: string
  password: string
  timePasswordChanged: number
}

export function writeLoginStore(
  dir: string,
  logins: FixtureLogin[],
  options: {
    primaryPassword?: string
    scheme?: 'pbes2' | '3des'
    /** Profile migrated to AES logins: an old 3DES key shares the key ID. */
    legacyKeyFirst?: boolean
  } = {},
): void {
  const primary = options.primaryPassword ?? ''
  const blob = options.scheme === '3des' ? tripleDesBlob : pbes2Blob
  const globalSalt = randomBytes(20)
  const loginKey = randomBytes(options.scheme === '3des' ? 24 : 32)
  const keyId = Buffer.from('f8000000000000000000000000000001', 'hex')
  const db = new Database(join(dir, 'key4.db'))
  db.run('CREATE TABLE metaData (id PRIMARY KEY, item1, item2)')
  db.run('CREATE TABLE nssPrivate (id PRIMARY KEY, a11, a102)')
  db.query('INSERT INTO metaData VALUES (?, ?, ?)').run(
    'password',
    globalSalt,
    blob(globalSalt, primary, Buffer.from('password-check\x02\x02', 'latin1')),
  )
  const insertKey = db.query('INSERT INTO nssPrivate VALUES (?, ?, ?)')
  if (options.legacyKeyFirst) {
    insertKey.run(1, tripleDesBlob(globalSalt, primary, randomBytes(24)), keyId)
  }
  insertKey.run(2, blob(globalSalt, primary, loginKey), keyId)
  // A non-login key (other CKA_ID) that must never be used for logins.
  insertKey.run(
    3,
    blob(globalSalt, primary, randomBytes(32)),
    Buffer.from('a5005a', 'hex'),
  )
  db.close()
  const cipher = options.scheme === '3des' ? '3des' : 'aes'
  const json = {
    nextId: logins.length + 1,
    version: 3,
    logins: logins.map((l, i) => ({
      id: i + 1,
      hostname: l.hostname,
      httpRealm: l.httpRealm ?? null,
      formSubmitURL: l.httpRealm ? null : l.hostname,
      usernameField: 'user',
      passwordField: 'pass',
      encryptedUsername: loginField(loginKey, l.username, cipher),
      encryptedPassword: loginField(loginKey, l.password, cipher),
      guid: l.guid,
      encType: 1,
      timeCreated: l.timePasswordChanged,
      timeLastUsed: l.timePasswordChanged,
      timePasswordChanged: l.timePasswordChanged,
      timesUsed: 1,
    })),
  }
  writeFileSync(join(dir, 'logins.json'), JSON.stringify(json))
}

export interface FixtureCookieRow {
  originAttributes?: string
  name: string
  value: string
  host: string
  path?: string
  expiry: number
  isSecure?: number
  isHttpOnly?: number
  sameSite?: number
  schemeMap?: number
}

export function writeCookieDb(dir: string, rows: FixtureCookieRow[]): void {
  const db = new Database(join(dir, 'cookies.sqlite'))
  db.run(
    "CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY, originAttributes TEXT NOT NULL DEFAULT '', name TEXT, value TEXT, host TEXT, path TEXT, expiry INTEGER, lastAccessed INTEGER, creationTime INTEGER, isSecure INTEGER, isHttpOnly INTEGER, inBrowserElement INTEGER DEFAULT 0, sameSite INTEGER DEFAULT 0, schemeMap INTEGER DEFAULT 0, isPartitionedAttributeSet INTEGER DEFAULT 0, updateTime INTEGER)",
  )
  const insert = db.query(
    'INSERT INTO moz_cookies (originAttributes, name, value, host, path, expiry, isSecure, isHttpOnly, sameSite, schemeMap) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
  )
  for (const r of rows) {
    insert.run(
      r.originAttributes ?? '',
      r.name,
      r.value,
      r.host,
      r.path ?? '/',
      r.expiry,
      r.isSecure ?? 0,
      r.isHttpOnly ?? 0,
      r.sameSite ?? 0,
      r.schemeMap ?? 2,
    )
  }
  db.close()
}

/** mozLz4 with one literal-only LZ4 block: valid input for any LZ4 decoder. */
export function mozLz4(text: string): Buffer {
  const data = Buffer.from(text, 'utf8')
  const lengthBytes: number[] = []
  if (data.length >= 15) {
    let rest = data.length - 15
    while (rest >= 255) {
      lengthBytes.push(255)
      rest -= 255
    }
    lengthBytes.push(rest)
  }
  const token = Math.min(data.length, 15) << 4
  const size = Buffer.alloc(4)
  size.writeUInt32LE(data.length)
  return Buffer.concat([
    Buffer.from('mozLz40\0', 'latin1'),
    size,
    Buffer.from([token, ...lengthBytes]),
    data,
  ])
}

export function writeSessionStore(dir: string, state: unknown): void {
  mkdirSync(join(dir, 'sessionstore-backups'), { recursive: true })
  writeFileSync(
    join(dir, 'sessionstore-backups', 'recovery.jsonlz4'),
    mozLz4(JSON.stringify(state)),
  )
}
