// The standard globals a Node Playwright script uses (`URL`, `URLSearchParams`, `Buffer`,
// `TextEncoder`, `TextDecoder`, `structuredClone`, `crypto`, `setInterval`) through Neo and through
// real Playwright on one BrowserOS neo: each script runs the same expressions in Neo's realm and
// in Playwright's Node, and the test compares the values, error kinds included (never wording).
// Random values are compared by shape. `setInterval` counts ticks its own callback reports, so no
// result depends on the machine's speed. A timer callback that throws is not compared here: Node
// exits on it (neo-runtime `tests/realm_globals.rs` pins Neo's run ending with that error).
//
// The REAL regression is staynb-1 (REAL_BENCH_TESTING results/staynb-1/feedback.md, item 2):
// `await page.goto(new URL(href, page.url()).href)` threw `ReferenceError: URL is not defined`.
//
// Not served, so not compared: `Intl` and `fetch` (neo-quickjs-injected `src/globals.ts` says why).
//
// Runs under Node, not Bun, on the shared harness (harness.mjs).

import { test } from 'node:test'
import { compareMethod, skip } from './harness.mjs'

/** Every expression's value, or the kind of error it threw. */
const VALUES = `const out = {}
const t = (name, f) => { try { out[name] = f() } catch (e) { out[name] = { threw: e.name, isTypeError: e instanceof TypeError } } }
t('types', () => ['URL','URLSearchParams','setInterval','clearInterval','Buffer','TextEncoder','TextDecoder','structuredClone','crypto'].map(n => typeof globalThis[n]))
t('rel', () => new URL('/x?a=1', 'https://h.test/p/').href)
t('rel2', () => new URL('../up?q#h', 'https://h.test/a/b/c').href)
t('parts', () => { const u = new URL('https://user:pw@A.test:8443/a%20b/c?x=1&y=a+b&x=2#frag'); return [u.href,u.origin,u.protocol,u.username,u.password,u.host,u.hostname,u.port,u.pathname,u.search,u.hash] })
t('instanceof', () => new URL('https://a.test') instanceof URL)
t('tag', () => Object.prototype.toString.call(new URL('https://a.test')))
t('json', () => JSON.stringify({ u: new URL('https://a.test/x') }))
t('invalid', () => new URL('not a url'))
t('invalidBase', () => new URL('/x', 'nope'))
t('canParse', () => [URL.canParse('https://a'), URL.canParse('x'), URL.canParse('/x', 'https://a/')])
t('parse', () => [URL.parse('x'), URL.parse('/y', 'https://a/').href])
t('setters', () => { const u = new URL('https://h.test/p?a=1#f'); u.pathname = '/a b'; u.search = '?b=2 3'; u.hash = 'z'; u.port = '8080'; u.hostname = 'o.test'; u.protocol = 'http'; u.username='me'; u.password='pw'; return [u.href, [...u.searchParams]] })
t('protoIgnored', () => { const u = new URL('https://h.test/'); u.protocol = 'foo'; u.port = 'abc'; return u.href })
t('hrefSet', () => { const u = new URL('https://h.test/'); u.href = 'http://x.test/?k=v'; return [u.href, u.searchParams.get('k')] })
t('hrefBad', () => { const u = new URL('https://h.test/'); u.href = 'nope' })
t('spLinked', () => { const u = new URL('https://h.test/p?a=1'); const sp = u.searchParams; sp.append('b', '2 3'); sp.set('a', 'x&y'); const h1 = u.href; sp.delete('a'); sp.delete('b'); return [h1, u.href, u.search, sp === u.searchParams] })
t('spSort', () => { const u = new URL('https://h.test/?c=3&a=1&b=2&a=0'); u.searchParams.sort(); return u.href })
t('spInit', () => [new URLSearchParams({a:'1 2', b:'é&'}).toString(), new URLSearchParams([['x','1'],['x','2']]).toString(), new URLSearchParams('?q=a+b&r=%C3%A9&s=%zz&t').toString(), [...new URLSearchParams('q=a+b&r=%C3%A9&s=%zz&t&=v&&')], new URLSearchParams(null).toString(), new URLSearchParams(new URLSearchParams('a=1')).toString()])
t('spMethods', () => { const p = new URLSearchParams('a=1&b=2&a=3'); return [p.get('a'), p.getAll('a'), p.has('b'), p.has('a','3'), p.has('a','4'), p.get('z'), p.size, [...p.keys()], [...p.values()], String(p)] })
t('spEncode', () => new URLSearchParams({ k: "!'()~*-._ /?&=+%é😀" }).toString())
t('spBadPair', () => new URLSearchParams([['a']]))
t('spLone', () => new URLSearchParams({ k: '\\ud800' }).toString())
t('enc', () => [...new TextEncoder().encode('héllo €😀')])
t('encLone', () => [...new TextEncoder().encode('a\\ud800b')])
t('dec', () => new TextDecoder().decode(new TextEncoder().encode('héllo €😀')))
t('decBad', () => new TextDecoder().decode(new Uint8Array([0x61, 0xff, 0xe2, 0x82, 0x62, 0xf0, 0x9f])))
t('decBOM', () => [new TextDecoder().decode(new Uint8Array([0xef,0xbb,0xbf,0x61])), new TextDecoder('utf-8', {ignoreBOM: true}).decode(new Uint8Array([0xef,0xbb,0xbf,0x61])).length])
t('decFatal', () => new TextDecoder('utf-8', { fatal: true }).decode(new Uint8Array([0xff])))
t('decStream', () => { const d = new TextDecoder(); const b = new TextEncoder().encode('€😀'); return [d.decode(b.subarray(0, 2), {stream:true}), d.decode(b.subarray(2, 5), {stream:true}), d.decode(b.subarray(5))] })
t('decLabel', () => new TextDecoder('UTF8').encoding)
t('encodeInto', () => { const d = new Uint8Array(5); const r = new TextEncoder().encodeInto('a€😀', d); return [r, [...d]] })
t('buf1', () => Buffer.from('héllo').toString('base64'))
t('buf2', () => Buffer.from('aMOpbGxv', 'base64').toString())
t('buf3', () => [Buffer.from('68656c6c6fzz', 'hex').toString(), Buffer.from('héllo', 'latin1').toString('hex'), Buffer.from('hi', 'utf16le').toString('hex'), Buffer.from('aGk_-w', 'base64url').toString('hex'), Buffer.from('aGk\\n=xx', 'base64').toString()])
t('buf4', () => [Buffer.from([1, 2, 256, -1]).toString('hex'), Buffer.from(new Uint16Array([1, 258])).toString('hex'), Buffer.from(new Uint8Array([7, 8]).buffer).toString('hex'), Buffer.from({type: 'Buffer', data: [104, 105]}).toString()])
t('bufView', () => { const ab = new ArrayBuffer(4); const b = Buffer.from(ab, 1, 2); b[0] = 9; return [b.length, new Uint8Array(ab)[1]] })
t('bufAlloc', () => [Buffer.alloc(3).toString('hex'), Buffer.alloc(5, 'ab').toString(), Buffer.alloc(3, 257).toString('hex'), Buffer.alloc(4, Buffer.from([1,2,3])).toString('hex')])
t('bufAllocBad', () => Buffer.alloc(-1))
t('bufConcat', () => [Buffer.concat([Buffer.from('ab'), new Uint8Array([99])]).toString(), Buffer.concat([Buffer.from('abc')], 5).toString('hex'), Buffer.concat([Buffer.from('abc')], 2).toString()])
t('bufLen', () => [Buffer.byteLength('héllo'), Buffer.byteLength('aGk=', 'base64'), Buffer.byteLength(new Uint8Array(7)), Buffer.from('😀').length])
t('bufIs', () => [Buffer.isBuffer(Buffer.from('a')), Buffer.isBuffer(new Uint8Array(1)), Buffer.isEncoding('UTF-8'), Buffer.isEncoding('nope'), Buffer.compare(Buffer.from('a'), Buffer.from('b'))])
t('bufJson', () => JSON.stringify(Buffer.from('hi')))
t('bufSub', () => { const b = Buffer.from('hello'); return [b.subarray(1, 3) instanceof Buffer, b.subarray(1,3).toString()] })
t('bufBadEnc', () => Buffer.from('a', 'nope'))
t('bufBadArg', () => Buffer.from(5))
t('clone', () => { const a = { d: new Date(0), m: new Map([[1, { x: [1, 2] }]]), s: new Set(['a']), r: /x/gi, n: null, u: undefined, big: 10n, nested: { deep: [1, { z: 2 }] } }; a.self = a; const c = structuredClone(a); return [c !== a, c.self === c, c.d instanceof Date, c.d.getTime(), c.m instanceof Map, c.m.get(1).x, [...c.s], c.r.source, c.r.flags, 'u' in c, typeof c.big, JSON.stringify(c.nested)] })
t('cloneTyped', () => { const buf = new ArrayBuffer(8); const a = new Uint8Array(buf, 2, 4); a[0] = 5; const v = { a, b: new Uint16Array(buf) }; const c = structuredClone(v); return [c.a instanceof Uint8Array, c.a.byteOffset, c.a[0], c.a.buffer === c.b.buffer, c.a.buffer !== buf, Object.prototype.toString.call(structuredClone(Buffer.from('x')))] })
t('cloneErr', () => { const e = structuredClone(new RangeError('boom')); return [e instanceof RangeError, e.message, e.name] })
t('cloneClass', () => { class P { constructor() { this.x = 1 } get y() { return 2 } } const c = structuredClone(new P()); return [c.constructor === Object, c.x, c.y] })
t('cloneFn', () => structuredClone({ f() {} }))
t('cloneFnName', () => { try { structuredClone(() => 1) } catch (e) { return [e.name, e instanceof DOMException, e.code] } })
t('cloneSym', () => structuredClone(Symbol('s')))
t('clonePromise', () => structuredClone(Promise.resolve()))
t('cloneArr', () => { const a = [1, , 3]; a.extra = 'e'; const c = structuredClone(a); return [c.length, 1 in c, c.extra] })
t('uuid', () => { const id = crypto.randomUUID(); return [/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(id), id !== crypto.randomUUID()] })
t('grv', () => { const a = new Uint32Array(8); const r = crypto.getRandomValues(a); return [r === a, a.some(x => x !== 0)] })
t('grvBig', () => crypto.getRandomValues(new Uint8Array(65537)))
t('grvBigName', () => { try { crypto.getRandomValues(new Uint8Array(65537)) } catch (e) { return e.name } })
t('grvFloat', () => { try { crypto.getRandomValues(new Float32Array(1)) } catch (e) { return e.name } })
t('cloneProto', () => { const c = structuredClone(JSON.parse('{"__proto__":{"p":1}}')); return [Object.keys(c), c.p === undefined, Object.getPrototypeOf(c) === Object.prototype] })
t('cloneURL', () => structuredClone(new URL('https://a.test/')))
t('cloneParams', () => structuredClone(new URLSearchParams('a=1')))
t('cloneEncoder', () => structuredClone(new TextEncoder()))
t('intervalNotFn', () => setInterval('x', 1))
t('timeoutNotFn', () => setTimeout(42, 1))
out.overflow = await new Promise((resolve) => {
  const id = setInterval(() => { clearInterval(id); resolve('ticked') }, 2 ** 32)
})
const ticks = []
let id
await new Promise((resolve) => {
  id = setInterval((step) => {
    ticks.push(step)
    if (ticks.length === 3) { clearInterval(id); resolve() }
  }, 5, 'tick')
})
await new Promise((resolve) => setTimeout(resolve, 100))
out.interval = ticks
let seen
await page.waitForURL((url) => {
  seen = [url instanceof URL, url.pathname, url.searchParams.get('who')]
  url.pathname = '/written'
  return true
})
out.predicate = seen
out.toPage = await page.evaluate((url) => [url instanceof URL, url.pathname, url.search], new URL('https://a.test/zz?q=1'))
const back = await page.evaluate(() => new URL('https://a.test/q?x=1'))
out.fromPage = [back instanceof URL, back.searchParams.get('x'), back.href]
const shot = await page.screenshot()
out.screenshot = [shot instanceof Buffer, Buffer.isBuffer(shot), shot.subarray(1, 4).toString(), Buffer.from(shot.toString('base64'), 'base64').equals(shot)]
return JSON.parse(JSON.stringify(out))`

/** A listing whose link to the stay is relative, as on https://evals-staynb.vercel.app. */
const LISTING = `<title>Stays</title>
<a href="stay/0?checkin=2026-12-01"><img alt="Cabin in the woods" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="></a>
<a href="stay/1"><img alt="Beach house" src="data:image/gif;base64,R0lGODlhAQABAAAAACw="></a>`

/** staynb-1's snippet, exactly, returning where the page landed. */
const STAYNB = `const href = await page.getByRole('link').filter({has: page.getByRole('img', {name: 'Cabin in the woods', exact: true})}).getAttribute('href');
await page.goto(new URL(href, page.url()).href);
return { href, url: page.url(), title: await page.title() }`

test('standard globals: Neo matches Playwright on Node', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await compareMethod(t, {
    html: '<title>globals</title><p>globals</p>',
    source: VALUES,
    probe: 'location.pathname',
  })
})

test('staynb-1: page.goto(new URL(href, page.url()).href) lands on the stay', {
  skip,
  timeout: 600_000,
}, async (t) => {
  await compareMethod(t, {
    html: LISTING,
    source: STAYNB,
    probe: 'location.pathname + location.search',
  })
})
