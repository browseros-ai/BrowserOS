/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Decrypts Firefox saved logins without loading Firefox's NSS library.
 *
 *   key4.db metaData('password')  -> proves the Primary Password (empty by default)
 *   key4.db nssPrivate.a11        -> the profile's login key, PBE-encrypted
 *   logins.json encrypted*        -> AES-256-CBC (current) or 3DES (old) under that key
 *
 * Every PBE blob is DER: SEQUENCE { AlgorithmIdentifier, OCTET STRING ciphertext }.
 * Firefox writes PBES2 (PBKDF2-HMAC-SHA256 + AES-256-CBC) since version 72;
 * the SHA1+3DES scheme stays for profiles created before that.
 */

import type { Database } from 'bun:sqlite'
import {
  createDecipheriv,
  createHash,
  createHmac,
  pbkdf2Sync,
} from 'node:crypto'
import {
  child,
  DER_TAG,
  type DerNode,
  decodeInteger,
  decodeOid,
  parseDer,
} from './der'

export const OID = {
  PBE_SHA1_3DES: '1.2.840.113549.1.12.5.1.3',
  PBES2: '1.2.840.113549.1.5.13',
  PBKDF2: '1.2.840.113549.1.5.12',
  HMAC_SHA1: '1.2.840.113549.2.7',
  HMAC_SHA256: '1.2.840.113549.2.9',
  AES256_CBC: '2.16.840.1.101.3.4.1.42',
  DES_EDE3_CBC: '1.2.840.113549.3.7',
} as const

/** CKA_ID NSS gives the login (SDR) key; other nssPrivate rows are not it. */
const LOGIN_KEY_ID = Buffer.from('f8000000000000000000000000000001', 'hex')
const PASSWORD_CHECK = 'password-check'

export class PrimaryPasswordError extends Error {
  constructor(public readonly attempted: boolean) {
    super(
      attempted
        ? 'The Firefox Primary Password is wrong.'
        : 'This Firefox profile has a Primary Password. Pass it with --primary-password-stdin.',
    )
  }
}

function sha1(...parts: Uint8Array[]): Buffer {
  const hash = createHash('sha1')
  for (const part of parts) hash.update(part)
  return hash.digest()
}

function hmacSha1(key: Uint8Array, data: Uint8Array): Buffer {
  return createHmac('sha1', key).update(data).digest()
}

function decipher(
  algorithm: 'aes-256-cbc' | 'des-ede3-cbc',
  key: Uint8Array,
  iv: Uint8Array,
  data: Uint8Array,
): Buffer {
  const d = createDecipheriv(algorithm, key, iv)
  return Buffer.concat([d.update(data), d.final()])
}

/** NSS's legacy PKCS#12-style derivation (pbeWithSha1AndTripleDES-CBC). */
function decryptSha1TripleDes(
  params: DerNode,
  globalSalt: Uint8Array,
  password: Uint8Array,
  ciphertext: Uint8Array,
): Buffer {
  const entrySalt = child(params, 0, DER_TAG.OCTET_STRING).value
  const hp = sha1(globalSalt, password)
  const pes = Buffer.alloc(20)
  Buffer.from(entrySalt).copy(pes)
  const chp = sha1(hp, entrySalt)
  const k1 = hmacSha1(chp, Buffer.concat([pes, entrySalt]))
  const tk = hmacSha1(chp, pes)
  const k2 = hmacSha1(chp, Buffer.concat([tk, entrySalt]))
  const k = Buffer.concat([k1, k2])
  return decipher('des-ede3-cbc', k.subarray(0, 24), k.subarray(-8), ciphertext)
}

function decryptPbes2(
  params: DerNode,
  globalSalt: Uint8Array,
  password: Uint8Array,
  ciphertext: Uint8Array,
): Buffer {
  const kdf = child(params, 0, DER_TAG.SEQUENCE)
  if (decodeOid(child(kdf, 0)) !== OID.PBKDF2) {
    throw new Error('Unsupported key derivation in key4.db')
  }
  const kdfParams = child(kdf, 1, DER_TAG.SEQUENCE)
  const salt = child(kdfParams, 0, DER_TAG.OCTET_STRING).value
  const iterations = decodeInteger(child(kdfParams, 1))
  let keyLength = 32
  let digest = 'sha1'
  for (const node of kdfParams.children.slice(2)) {
    if (node.tag === DER_TAG.INTEGER) keyLength = decodeInteger(node)
    if (node.tag === DER_TAG.SEQUENCE) {
      digest = decodeOid(child(node, 0)) === OID.HMAC_SHA256 ? 'sha256' : 'sha1'
    }
  }
  const scheme = child(params, 1, DER_TAG.SEQUENCE)
  if (decodeOid(child(scheme, 0)) !== OID.AES256_CBC) {
    throw new Error('Unsupported cipher in key4.db')
  }
  let iv = child(scheme, 1, DER_TAG.OCTET_STRING).value
  // NSS stores a 14-byte IV and treats its own DER header (04 0E) as the
  // first two IV bytes. Every PBES2 key4.db Firefox wrote depends on this.
  if (iv.length === 14) iv = Buffer.concat([Buffer.from([0x04, 0x0e]), iv])
  const key = pbkdf2Sync(
    sha1(globalSalt, password),
    salt,
    iterations,
    keyLength,
    digest,
  )
  return decipher('aes-256-cbc', key, iv, ciphertext)
}

/** Decrypts one key4.db PBE blob; throws when the password does not fit. */
export function decryptPbeBlob(
  blob: Uint8Array,
  globalSalt: Uint8Array,
  password: string,
): Buffer {
  const root = parseDer(blob)
  const algorithm = child(root, 0, DER_TAG.SEQUENCE)
  const ciphertext = child(root, 1, DER_TAG.OCTET_STRING).value
  const oid = decodeOid(child(algorithm, 0))
  const params = child(algorithm, 1, DER_TAG.SEQUENCE)
  const pw = Buffer.from(password, 'utf8')
  if (oid === OID.PBES2) return decryptPbes2(params, globalSalt, pw, ciphertext)
  if (oid === OID.PBE_SHA1_3DES) {
    return decryptSha1TripleDes(params, globalSalt, pw, ciphertext)
  }
  throw new Error(`Unsupported key4.db algorithm ${oid}`)
}

/**
 * Returns the login keys from an opened key4.db snapshot. An empty Primary
 * Password is tried first, as Firefox does when none is set.
 *
 * A profile that predates Firefox's switch of logins to AES-256 holds two
 * keys under the same CKA_ID: the old 24-byte 3DES key and the new 32-byte
 * AES key. Both are returned; decryptLoginField picks by the entry's cipher.
 */
export function readLoginKeys(
  db: Database,
  primaryPassword?: string,
): Buffer[] {
  const meta = db
    .query("SELECT item1, item2 FROM metaData WHERE id = 'password'")
    .get() as { item1: Uint8Array; item2: Uint8Array } | null
  if (!meta) throw new Error('key4.db has no password metadata')
  const password = primaryPassword ?? ''
  let check: Buffer
  try {
    check = decryptPbeBlob(meta.item2, meta.item1, password)
  } catch {
    throw new PrimaryPasswordError(primaryPassword !== undefined)
  }
  if (!check.toString('latin1').startsWith(PASSWORD_CHECK)) {
    throw new PrimaryPasswordError(primaryPassword !== undefined)
  }
  const rows = db
    .query('SELECT a11, a102 FROM nssPrivate WHERE a11 IS NOT NULL')
    .all() as { a11: Uint8Array; a102: Uint8Array | null }[]
  const tagged = rows.filter(
    (r) => r.a102 && LOGIN_KEY_ID.equals(Buffer.from(r.a102)),
  )
  const keys = (tagged.length ? tagged : rows).map((r) =>
    decryptPbeBlob(r.a11, meta.item1, password),
  )
  if (keys.length === 0) throw new Error('key4.db holds no login key')
  return keys
}

const CIPHERS = {
  [OID.AES256_CBC]: { algorithm: 'aes-256-cbc', keyLength: 32 },
  [OID.DES_EDE3_CBC]: { algorithm: 'des-ede3-cbc', keyLength: 24 },
} as const

/**
 * Decrypts a logins.json `encryptedUsername` / `encryptedPassword` value:
 * SEQUENCE { OCTET STRING keyId, SEQUENCE { OID cipher, OCTET STRING iv }, OCTET STRING data }.
 */
export function decryptLoginField(base64: string, loginKeys: Buffer[]): string {
  const root = parseDer(Buffer.from(base64, 'base64'))
  const cipherNode = child(root, 1, DER_TAG.SEQUENCE)
  const oid = decodeOid(child(cipherNode, 0))
  const iv = child(cipherNode, 1, DER_TAG.OCTET_STRING).value
  const data = child(root, 2, DER_TAG.OCTET_STRING).value
  const cipher = CIPHERS[oid as keyof typeof CIPHERS]
  if (!cipher) throw new Error(`Unsupported login cipher ${oid}`)
  // Key length identifies the key: a wrong-length key never reaches the
  // cipher, so a lucky padding match cannot yield garbage plaintext.
  const candidates = loginKeys.filter((k) => k.length === cipher.keyLength)
  let lastError: unknown = new Error(`No ${cipher.keyLength}-byte login key`)
  for (const key of candidates) {
    try {
      return decipher(cipher.algorithm, key, iv, data).toString('utf8')
    } catch (error) {
      lastError = error
    }
  }
  throw lastError
}
