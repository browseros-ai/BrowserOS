/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The smallest DER reader NSS's on-disk structures need. key4.db and
 * logins.json wrap every secret in ASN.1 (algorithm identifiers, salts, IVs,
 * ciphertexts); this reads definite-length TLVs only, which is all NSS writes.
 */

export const DER_TAG = {
  INTEGER: 0x02,
  OCTET_STRING: 0x04,
  NULL: 0x05,
  OID: 0x06,
  SEQUENCE: 0x30,
} as const

export interface DerNode {
  tag: number
  /** Content bytes, without the tag and length header. */
  value: Uint8Array
  /** Parsed content of constructed nodes (SEQUENCE, SET, context tags). */
  children: DerNode[]
}

function readNode(
  buf: Uint8Array,
  offset: number,
): { node: DerNode; end: number } {
  if (offset + 2 > buf.length) throw new Error('DER: truncated header')
  const tag = buf[offset]
  let length = buf[offset + 1]
  let cursor = offset + 2
  if (length & 0x80) {
    const lengthBytes = length & 0x7f
    if (lengthBytes === 0 || lengthBytes > 4) {
      throw new Error('DER: unsupported length form')
    }
    length = 0
    for (let i = 0; i < lengthBytes; i++) {
      length = length * 256 + buf[cursor + i]
    }
    cursor += lengthBytes
  }
  const end = cursor + length
  if (end > buf.length) throw new Error('DER: content exceeds buffer')
  const value = buf.subarray(cursor, end)
  const constructed = (tag & 0x20) !== 0
  return {
    node: { tag, value, children: constructed ? readAll(value) : [] },
    end,
  }
}

function readAll(buf: Uint8Array): DerNode[] {
  const nodes: DerNode[] = []
  let offset = 0
  while (offset < buf.length) {
    const { node, end } = readNode(buf, offset)
    nodes.push(node)
    offset = end
  }
  return nodes
}

export function parseDer(buf: Uint8Array): DerNode {
  return readNode(buf, 0).node
}

export function decodeOid(node: DerNode): string {
  if (node.tag !== DER_TAG.OID) throw new Error('DER: expected an OID')
  const bytes = node.value
  const parts = [Math.floor(bytes[0] / 40), bytes[0] % 40]
  let current = 0
  for (let i = 1; i < bytes.length; i++) {
    current = current * 128 + (bytes[i] & 0x7f)
    if (!(bytes[i] & 0x80)) {
      parts.push(current)
      current = 0
    }
  }
  return parts.join('.')
}

export function decodeInteger(node: DerNode): number {
  if (node.tag !== DER_TAG.INTEGER) throw new Error('DER: expected INTEGER')
  let result = 0
  for (const byte of node.value) result = result * 256 + byte
  return result
}

/** Child accessor that fails loudly: a missing field means an unknown format. */
export function child(node: DerNode, index: number, tag?: number): DerNode {
  const found = node.children[index]
  if (!found || (tag !== undefined && found.tag !== tag)) {
    throw new Error(`DER: unexpected structure at child ${index}`)
  }
  return found
}
