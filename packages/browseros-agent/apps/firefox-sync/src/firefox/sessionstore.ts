/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Reads session cookies (cookies without an expiry) from Firefox's session
 * store. Firefox keeps them out of cookies.sqlite and saves them with the
 * window state instead: `sessionstore-backups/recovery.jsonlz4` while Firefox
 * runs, `sessionstore.jsonlz4` after a clean exit. Both use Mozilla's mozLz4
 * framing: the magic `mozLz40\0`, a little-endian uint32 decompressed size,
 * then one raw LZ4 block.
 */

import { existsSync, readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'
import {
  type CookieReadResult,
  type FirefoxCookie,
  sameSiteFromFirefox,
} from './cookies'

const MOZ_LZ4_MAGIC = 'mozLz40\0'

/** Decodes a raw LZ4 block (no frame header), as written by mozLz4. */
export function lz4BlockDecompress(
  src: Uint8Array,
  outputSize: number,
): Uint8Array {
  const out = new Uint8Array(outputSize)
  let i = 0
  let o = 0
  const readLength = (base: number) => {
    let length = base
    if (base === 15) {
      let byte: number
      do {
        byte = src[i++]
        length += byte
      } while (byte === 255 && i < src.length)
    }
    return length
  }
  while (i < src.length) {
    const token = src[i++]
    const literals = readLength(token >> 4)
    if (o + literals > out.length || i + literals > src.length) {
      throw new Error('LZ4: literal run exceeds bounds')
    }
    out.set(src.subarray(i, i + literals), o)
    i += literals
    o += literals
    if (i >= src.length) break
    const offset = src[i] | (src[i + 1] << 8)
    i += 2
    if (offset === 0 || offset > o) throw new Error('LZ4: invalid match offset')
    const matchLength = readLength(token & 0x0f) + 4
    if (o + matchLength > out.length)
      throw new Error('LZ4: match exceeds bounds')
    // Byte-by-byte on purpose: matches may overlap their own output.
    for (let k = 0; k < matchLength; k++, o++) out[o] = out[o - offset]
  }
  return out.subarray(0, o)
}

export function decodeMozLz4(buf: Uint8Array): string {
  const magic = Buffer.from(buf.subarray(0, 8)).toString('latin1')
  if (magic !== MOZ_LZ4_MAGIC) throw new Error('Not a mozLz4 file')
  const size = new DataView(buf.buffer, buf.byteOffset + 8, 4).getUint32(
    0,
    true,
  )
  return Buffer.from(lz4BlockDecompress(buf.subarray(12), size)).toString(
    'utf8',
  )
}

interface SessionCookie {
  host: string
  name: string
  value?: string
  path?: string
  secure?: boolean
  httponly?: boolean
  sameSite?: number
  schemeMap?: number
  originAttributes?: {
    userContextId?: number
    privateBrowsingId?: number
    partitionKey?: string
  }
}

/** The newest session file: live recovery data beats a clean-exit snapshot. */
function sessionFile(profileDir: string): string | undefined {
  const candidates = [
    join(profileDir, 'sessionstore-backups', 'recovery.jsonlz4'),
    join(profileDir, 'sessionstore.jsonlz4'),
  ].filter((path) => existsSync(path))
  return candidates.sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0]
}

export function parseSessionCookies(json: string): CookieReadResult {
  const state = JSON.parse(json) as { cookies?: SessionCookie[] }
  const result: CookieReadResult = { cookies: [], skipped: {} }
  for (const c of state.cookies ?? []) {
    const attrs = c.originAttributes ?? {}
    const reason = attrs.partitionKey
      ? 'partitioned'
      : attrs.userContextId
        ? 'container'
        : attrs.privateBrowsingId
          ? 'private'
          : undefined
    if (reason) {
      result.skipped[reason] = (result.skipped[reason] ?? 0) + 1
      continue
    }
    const cookie: FirefoxCookie = {
      name: c.name,
      value: c.value ?? '',
      host: c.host,
      path: c.path || '/',
      expiresMs: null,
      secure: c.secure === true,
      httpOnly: c.httponly === true,
      sameSite: sameSiteFromFirefox(c.sameSite),
      schemeMap: c.schemeMap ?? 0,
    }
    result.cookies.push(cookie)
  }
  return result
}

export function readSessionCookies(profileDir: string): CookieReadResult {
  const file = sessionFile(profileDir)
  if (!file) return { cookies: [], skipped: {} }
  return parseSessionCookies(decodeMozLz4(readFileSync(file)))
}
