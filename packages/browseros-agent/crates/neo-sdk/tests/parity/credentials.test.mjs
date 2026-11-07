// `context.credentials` (`install`, `create`, `get`, `delete`) through Neo and through Playwright
// on one BrowserOS neo: a credential the script seeds signs a real `navigator.credentials.get`
// once installed (its id, its user handle, a signature its own public key verifies), each
// assertion moves its count, a credential the page creates is listed, a deleted one is gone, and
// a count out of range fails. Without `install()` the page's own WebAuthn answers. Playwright's
// `install()` reaches every page of the browser's default context, a tab Neo runs on included,
// so each case disconnects Playwright before Neo's turn: its init script and binding go with its
// sessions, and the case reloads its page after installing. Neo serves the credentials through Chromium's virtual authenticator, so a credential
// the page creates starts counting at 1 where Playwright's starts at 0: asserted on each side
// (neo-runtime `credentials.rs`).
//
// Playwright cases (pin 26137face): `tests/library/browsercontext-webauthn.spec.ts` "AU-017 should
// not intercept navigator.credentials without install()", "AU-018 should seed a known credential
// and authenticate", "AU-019 should capture a page-created credential and reuse it in another
// context" (its first half), "AU-021 should seed and report signCount" and "AU-022 should reject
// invalid signCount". WebAuthn needs a domain, so the cases run on `localhost`.

import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const pages = { '/webauthn.html': '<title>webauthn</title>' }

const SETUP = `const origin = page.url().slice(0, page.url().indexOf('/', 'http://'.length)).replace('127.0.0.1', 'localhost')
await page.goto(origin + '/webauthn.html')
/** A ceremony in the page: the assertion's id, user handle, count and whether its signature verifies. */
const assertWith = (credential) => page.evaluate(async ({ id, publicKey }) => {
  const bytes = (s) => Uint8Array.from(atob(s.replace(/-/g, '+').replace(/_/g, '/') + '='.repeat((4 - s.length % 4) % 4)), (c) => c.charCodeAt(0))
  const text = (b) => btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\\+/g, '-').replace(/\\//g, '_').replace(/=+$/, '')
  const cred = await navigator.credentials.get({ publicKey: {
    challenge: crypto.getRandomValues(new Uint8Array(32)), rpId: 'localhost', userVerification: 'preferred',
    allowCredentials: id ? [{ type: 'public-key', id: bytes(id) }] : [],
  } })
  const response = cred.response
  const data = new Uint8Array(response.authenticatorData)
  let verified = null
  if (publicKey) {
    const key = await crypto.subtle.importKey('spki', bytes(publicKey), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['verify'])
    const signed = new Uint8Array([...data, ...new Uint8Array(await crypto.subtle.digest('SHA-256', response.clientDataJSON))])
    // WebAuthn signatures are DER; WebCrypto verifies raw r||s.
    const der = new Uint8Array(response.signature)
    const int = (at) => { const length = der[at + 1]; let value = der.slice(at + 2, at + 2 + length); while (value.length > 32 && value[0] === 0) value = value.slice(1); const out = new Uint8Array(32); out.set(value, 32 - value.length); return [out, at + 2 + length] }
    const [r, next] = int(2)
    const [s] = int(next)
    verified = await crypto.subtle.verify({ name: 'ECDSA', hash: 'SHA-256' }, key, new Uint8Array([...r, ...s]), signed)
  }
  return { id: cred.id, type: cred.type, userHandle: response.userHandle ? text(response.userHandle) : null, signCount: new DataView(data.buffer).getUint32(33), verified }
}, credential)
`

const cases = [
  {
    name: 'without install(), the page keeps its own WebAuthn',
    source: `${SETUP}const seeded = await context.credentials.create('localhost')
const answer = await page.evaluate(async () => {
  const controller = new AbortController()
  setTimeout(() => controller.abort(), 1500)
  try {
    await navigator.credentials.get({ signal: controller.signal, publicKey: { challenge: new Uint8Array(32), rpId: 'localhost' } })
    return 'answered'
  } catch (error) { return error.name }
})
await context.credentials.delete(seeded.id)
return { answer, installed: await page.evaluate(() => globalThis.__pwWebAuthnInstalled === true) }`,
    expected: { answer: 'AbortError', installed: false },
  },
  {
    name: 'a seeded credential signs an assertion once installed',
    source: `${SETUP}const known = await context.credentials.create('localhost')
await context.credentials.install()
await page.reload()
const asserted = await assertWith(known)
const [listed] = await context.credentials.get({ id: known.id })
await context.credentials.delete(known.id)
return {
  shape: Object.keys(known).sort(),
  sameId: asserted.id === known.id,
  type: asserted.type,
  sameUser: asserted.userHandle === known.userHandle,
  signCount: asserted.signCount,
  verified: asserted.verified,
  listedCount: listed.signCount,
}`,
    expected: {
      shape: [
        'id',
        'privateKey',
        'publicKey',
        'rpId',
        'signCount',
        'userHandle',
      ],
      sameId: true,
      type: 'public-key',
      sameUser: true,
      signCount: 1,
      verified: true,
      listedCount: 1,
    },
  },
  {
    name: 'a seeded count moves with each assertion; one out of range fails',
    source: `${SETUP}await context.credentials.install()
await page.reload()
const known = await context.credentials.create('localhost', { signCount: 7 })
const first = await assertWith({ id: known.id })
const second = await assertWith({ id: known.id })
const [listed] = await context.credentials.get({ id: known.id })
const invalid = await context.credentials.create('localhost', { signCount: -1 }).then(() => 'created', (error) => error.message.split('\\n')[0])
await context.credentials.delete(known.id)
return { counts: [first.signCount, second.signCount, listed.signCount], invalid }`,
    expected: {
      counts: [8, 9, 9],
      invalid:
        'credentials.create: signCount must be between 0 and 4294967295, got -1',
    },
  },
  {
    name: 'a deleted credential is gone',
    source: `${SETUP}const known = await context.credentials.create('localhost')
const before = (await context.credentials.get({ rpId: 'localhost' })).some((c) => c.id === known.id)
await context.credentials.delete(known.id)
return { before, after: await context.credentials.get({ id: known.id }) }`,
    expected: { before: true, after: [] },
  },
]

test('context.credentials: Neo matches Playwright', { skip }, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo, connect }) => {
    for (const { name, source, expected } of cases) {
      const reference = outcomeValue(await playwright('/webauthn.html', source))
      await (await connect()).close()
      assert.deepEqual(reference, expected, `${name}: Playwright`)
      assert.deepEqual(
        outcomeValue(await neo('/webauthn.html', source)),
        reference,
        `${name}: Neo`,
      )
    }
  })
})

test('context.credentials: a credential the page creates is listed', {
  skip,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo, connect }) => {
    const source = `${SETUP}await context.credentials.install()
await page.reload()
const created = await page.evaluate(async () => {
  const cred = await navigator.credentials.create({ publicKey: {
    challenge: new Uint8Array(32), rp: { id: 'localhost', name: 'fixture' },
    user: { id: new Uint8Array([1, 2, 3]), name: 'user', displayName: 'User' },
    pubKeyCredParams: [{ type: 'public-key', alg: -7 }],
    authenticatorSelection: { residentKey: 'required', userVerification: 'preferred' },
  } })
  return cred.id
})
const listed = (await context.credentials.get({ rpId: 'localhost' })).find((c) => c.id === created)
if (listed) await context.credentials.delete(listed.id)
return listed && { rpId: listed.rpId, userHandle: listed.userHandle, hasKeys: !!(listed.privateKey && listed.publicKey), signCount: listed.signCount }`
    const reference = outcomeValue(await playwright('/webauthn.html', source))
    await (await connect()).close()
    const listed = { rpId: 'localhost', userHandle: 'AQID', hasKeys: true }
    assert.deepEqual(reference, { ...listed, signCount: 0 }, 'Playwright')
    // Chromium's virtual authenticator counts the creation itself.
    assert.deepEqual(
      outcomeValue(await neo('/webauthn.html', source)),
      { ...listed, signCount: 1 },
      'Neo',
    )
  })
})
