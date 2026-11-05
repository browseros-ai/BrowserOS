// A frameLocator chain four levels deep into the nested fixture (b1, c2, a3, c4: each an
// out-of-process frame), read at every level. F8 clicks through the same chain.
await page.goto(fixtures.url('nest/top.html'))
const b1 = page.frameLocator('iframe[title=b1]')
const c2 = b1.frameLocator('iframe[title=c2]')
const a3 = c2.frameLocator('iframe[title=a3]')
const c4 = a3.frameLocator('iframe[title=c4]')
const read = {}
for (const [name, frame] of Object.entries({ b1, c2, a3, c4 }))
  read[name] = await harness.settle(frame.getByRole('button').textContent())
read.c4Field = await harness.settle(c4.getByRole('textbox').count())
read.strictInC2 = await harness.settle(c2.locator('button, input').textContent())
return read
