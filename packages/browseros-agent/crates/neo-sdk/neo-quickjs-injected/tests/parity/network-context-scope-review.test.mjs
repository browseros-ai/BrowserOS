// Neo: a context's network events exclude pages belonging to another context.
import assert from 'node:assert/strict'
import { test } from 'node:test'
import { skip } from './harness.mjs'
import { outcomeValue, withPlatform } from './platform.mjs'

const paths = ['/own', '/foreign-existing', '/foreign-new']
const pages = Object.fromEntries(
  ['/start', ...paths].map((path) => [path, '<title>scope</title>']),
)
const source = `const origin=page.url().slice(0,page.url().indexOf('/','http://'.length))
const created=await context.browser().newContext()
const before=await created.newPage()
const requests=[],pageRequests=[]
const pageListener=request=>{
  const path=request.url().slice(origin.length)
  if(['/foreign-existing','/foreign-new'].includes(path))pageRequests.push(path)
}
before.on('request',pageListener)
const listener=request=>{
  const path=request.url().slice(origin.length)
  if(['/own','/foreign-existing','/foreign-new'].includes(path))requests.push(path)
}
context.on('request',listener)
const response=await before.goto(origin+'/foreign-existing')
const after=await created.newPage()
after.on('request',pageListener)
await after.goto(origin+'/foreign-new')
await page.goto(origin+'/own')
await page.title()
context.off('request',listener)
const result={requests:requests.sort(),pageRequests:pageRequests.sort(),foreignStatus:response.status(),contextPreserved:before.context()===created}
await created.close()
return result`

test('context request listeners exclude existing and later pages of a created context', {
  skip,
  timeout: 180000,
}, async (t) => {
  await withPlatform(t, { pages }, async ({ playwright, neo, connect }) => {
    const reference = outcomeValue(await playwright('/start', source))
    await (await connect()).close()
    const actual = outcomeValue(await neo('/start', source))
    t.diagnostic(JSON.stringify({ playwright: reference, neo: actual }))
    assert.deepEqual(reference, {
      requests: ['/own'],
      pageRequests: ['/foreign-existing', '/foreign-new'],
      foreignStatus: 200,
      contextPreserved: true,
    })
    assert.deepEqual(actual, reference)
  })
})
