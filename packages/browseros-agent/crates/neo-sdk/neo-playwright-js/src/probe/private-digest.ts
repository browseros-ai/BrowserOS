/**
 * The mark follows the value (protected-fill ruling R50). The mark on a box Neo typed a private
 * value into is by element (`private-entry.ts`), so a box the page re-creates or clones, or a
 * copy of the value in another box, would print the value. So at each read-back this world also
 * keeps a keyed digest of what the box held, with whitespace, dashes and slashes set aside, and
 * any text control of this document that holds a kept value is masked as a marked box is.
 *
 * - The key is 128 random bits, made in this isolated world the first time it keeps a digest. It
 *   lives in this module's state and dies with the document. The page's scripts cannot reach it,
 *   Rust never has it, and neither the key nor a digest is in any answer the page side gives.
 * - The digest is SipHash-2-4, a keyed function made for short inputs, over the set-aside text's
 *   UTF-16 code units: written here with 32-bit halves, because the browser's own digest call
 *   answers later and the mask cannot wait. 64 bits out, kept with the set-aside length, so only
 *   a control whose set-aside text has a kept length is digested at all.
 * - It must not become a way to guess. At the start of every read Neo makes through the mask
 *   (`noteHolders`), a digest that no connected text control of the document holds is forgotten.
 *   Once the value has left the page, a script that sets a box to a guess and reads it gets at
 *   most one answer in bullets, not thousands.
 */

/** This world's key, four 32-bit words; made when the first digest is kept. */
let key: Uint32Array | undefined

/** The kept digests, each with its set-aside text's length. */
const kept = new Map<string, number>()

/** The controls that held a kept value at the start of the current read through the mask. */
let holders = new WeakSet<Element>()

/** The characters a compare sets aside: whitespace, dashes and slashes, as `spaced` does. */
export function setAside(text: string): string {
  return text.replace(/[\s\-/]/g, '')
}

/** Keeps a digest of `text`; a text that is empty once set aside keeps none. */
export function keepDigest(text: string): void {
  const bare = setAside(text)
  if (!bare) return
  key ??= crypto.getRandomValues(new Uint32Array(4))
  kept.set(digest(key, bare), bare.length)
}

/** Whether this world keeps any digest. */
export function keepsDigests(): boolean {
  return kept.size > 0
}

/**
 * The scan at the start of every read through the mask, before the mask is in place, so the
 * native values are read: notes the text controls of `document` that hold a kept value, and
 * forgets every digest none of them holds. It runs whenever a digest is kept, also when the read
 * touches no holder: that is what lets the first wrong guess forget a value that has left.
 */
export function noteHolders(document: Document): void {
  holders = new WeakSet()
  if (!key || kept.size === 0) return
  const lengths = new Set(kept.values())
  const held = new Set<string>()
  for (const control of textControls(document)) {
    const bare = setAside(control.value)
    if (!lengths.has(bare.length)) continue
    const found = digest(key, bare)
    if (!kept.has(found)) continue
    held.add(found)
    holders.add(control)
  }
  for (const known of kept.keys()) if (!held.has(known)) kept.delete(known)
}

/** Whether `element` held a kept value at the start of the current read through the mask. */
export function holdsKeptValue(element: Element): boolean {
  return holders.has(element)
}

/** The input types whose value is not text a person typed. */
const NOT_TEXT = new Set([
  'button',
  'checkbox',
  'color',
  'file',
  'image',
  'radio',
  'range',
  'reset',
  'submit',
])

type TextControl = HTMLInputElement | HTMLTextAreaElement

/**
 * Every connected `<input>` that holds text, `type=hidden` included, and every `<textarea>` of
 * `document`, down open shadow roots and the closed ones this world's walk has seen (its
 * `shadowRoot` getter answers those, shadow-roots.ts). A detached control is no holder.
 */
function* textControls(document: Document): Generator<TextControl> {
  const roots: (Document | ShadowRoot)[] = [document]
  for (const root of roots) {
    for (const element of root.querySelectorAll('*')) {
      if (element instanceof HTMLTextAreaElement) yield element
      else if (
        element instanceof HTMLInputElement &&
        !NOT_TEXT.has(element.type)
      )
        yield element
      const shadow = element.shadowRoot
      if (shadow) roots.push(shadow)
    }
  }
}

/** SipHash-2-4 of `text`'s UTF-16 code units (little-endian) under `secret`, as `hi:lo` hex. */
export function digest(secret: Uint32Array, text: string): string {
  const bytes = new Uint8Array(text.length * 2)
  for (let index = 0; index < text.length; index++) {
    const unit = text.charCodeAt(index)
    bytes[index * 2] = unit & 0xff
    bytes[index * 2 + 1] = unit >>> 8
  }
  const [hash0, hash1] = sipHash24(secret, bytes)
  return `${hash1.toString(16)}:${hash0.toString(16)}`
}

/**
 * SipHash-2-4 over `bytes` under the 128-bit `secret` (four little-endian 32-bit words, k0 low
 * then high, k1 low then high). Each 64-bit lane is a low and a high 32-bit half. Answers the
 * 64-bit result as its low and high halves.
 */
export function sipHash24(
  secret: Uint32Array,
  bytes: Uint8Array,
): [number, number] {
  const k0lo = secret[0] ?? 0
  const k0hi = secret[1] ?? 0
  const k1lo = secret[2] ?? 0
  const k1hi = secret[3] ?? 0
  // v0..v3 as [lo, hi] pairs, seeded with "somepseudorandomlygeneratedbytes".
  const v = new Uint32Array([
    (k0lo ^ 0x70736575) >>> 0,
    (k0hi ^ 0x736f6d65) >>> 0,
    (k1lo ^ 0x6e646f6d) >>> 0,
    (k1hi ^ 0x646f7261) >>> 0,
    (k0lo ^ 0x6e657261) >>> 0,
    (k0hi ^ 0x6c796765) >>> 0,
    (k1lo ^ 0x79746573) >>> 0,
    (k1hi ^ 0x74656462) >>> 0,
  ])
  const word = (at: number) =>
    ((bytes[at] ?? 0) |
      ((bytes[at + 1] ?? 0) << 8) |
      ((bytes[at + 2] ?? 0) << 16) |
      ((bytes[at + 3] ?? 0) << 24)) >>>
    0
  const absorb = (lo: number, hi: number) => {
    v[6] = (v[6] ?? 0) ^ lo
    v[7] = (v[7] ?? 0) ^ hi
    round(v)
    round(v)
    v[0] = (v[0] ?? 0) ^ lo
    v[1] = (v[1] ?? 0) ^ hi
  }
  const whole = bytes.length - (bytes.length % 8)
  for (let at = 0; at < whole; at += 8) absorb(word(at), word(at + 4))
  // The last block: what is left, and the length's low byte at the top.
  const last = new Uint8Array(8)
  last.set(bytes.subarray(whole))
  last[7] = bytes.length & 0xff
  const lastLo =
    ((last[0] ?? 0) |
      ((last[1] ?? 0) << 8) |
      ((last[2] ?? 0) << 16) |
      ((last[3] ?? 0) << 24)) >>>
    0
  const lastHi =
    ((last[4] ?? 0) |
      ((last[5] ?? 0) << 8) |
      ((last[6] ?? 0) << 16) |
      ((last[7] ?? 0) << 24)) >>>
    0
  absorb(lastLo, lastHi)
  v[4] = (v[4] ?? 0) ^ 0xff
  round(v)
  round(v)
  round(v)
  round(v)
  const lo = ((v[0] ?? 0) ^ (v[2] ?? 0) ^ (v[4] ?? 0) ^ (v[6] ?? 0)) >>> 0
  const hi = ((v[1] ?? 0) ^ (v[3] ?? 0) ^ (v[5] ?? 0) ^ (v[7] ?? 0)) >>> 0
  return [lo, hi]
}

/** One SipRound on the four lanes held as [lo, hi] pairs in `v`. */
function round(v: Uint32Array): void {
  add(v, 0, 2)
  rotate(v, 2, 13)
  xor(v, 2, 0)
  rotate(v, 0, 32)
  add(v, 4, 6)
  rotate(v, 6, 16)
  xor(v, 6, 4)
  add(v, 0, 6)
  rotate(v, 6, 21)
  xor(v, 6, 0)
  add(v, 4, 2)
  rotate(v, 2, 17)
  xor(v, 2, 4)
  rotate(v, 4, 32)
}

/** Lane at `into` += lane at `from`, modulo 2^64. */
function add(v: Uint32Array, into: number, from: number): void {
  const lo = (v[into] ?? 0) + (v[from] ?? 0)
  const carry = lo > 0xffffffff ? 1 : 0
  v[into] = lo >>> 0
  v[into + 1] = ((v[into + 1] ?? 0) + (v[from + 1] ?? 0) + carry) >>> 0
}

/** Lane at `into` ^= lane at `from`. */
function xor(v: Uint32Array, into: number, from: number): void {
  v[into] = ((v[into] ?? 0) ^ (v[from] ?? 0)) >>> 0
  v[into + 1] = ((v[into + 1] ?? 0) ^ (v[from + 1] ?? 0)) >>> 0
}

/** Lane at `at` rotated left by `bits` (13, 16, 17, 21 or 32). */
function rotate(v: Uint32Array, at: number, bits: number): void {
  const lo = v[at] ?? 0
  const hi = v[at + 1] ?? 0
  if (bits === 32) {
    v[at] = hi
    v[at + 1] = lo
    return
  }
  v[at] = ((lo << bits) | (hi >>> (32 - bits))) >>> 0
  v[at + 1] = ((hi << bits) | (lo >>> (32 - bits))) >>> 0
}
