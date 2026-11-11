// Frame objects survive a renderer change; pending reads follow the frame, while a removed
// frame and an evaluate whose document disappeared must reject. Expectations are fixture facts.
await page.goto(`${origin}/frames.html`)
await expect(
  page.frameLocator('iframe[name=same]').locator('#name'),
).toHaveText('Same')
await expect(
  page.frameLocator('iframe[name=cross]').locator('#name'),
).toHaveText('Cross')
const same = page.frame('same'),
  cross = page.frame('cross')
for (const [frame, name, host, label] of [
  [cross, 'cross', '127.0.0.1', 'BackToParent'],
  [same, 'same', 'localhost', 'ToOtherSite'],
  [cross, 'cross', '[::1]', 'ThirdSite'],
]) {
  const target = `http://${host}:${new URL(origin).port}/child.html?name=${label}`
  await page.evaluate(
    ({ name, target }) => {
      setTimeout(() => {
        document.querySelector(`iframe[name=${name}]`).src = target
      }, 200)
    },
    { name, target },
  )
  await frame.waitForURL(target, { timeout: 10000 })
  expect(frame.url()).toBe(target)
  expect(page.frames().includes(frame)).toBe(true)
  expect(await frame.locator('#name').textContent()).toBe(label)
}
await page.evaluate(() => {
  const f = document.createElement('iframe')
  f.name = 'late'
  f.src = '/child.html?name=Late'
  document.body.append(f)
})
await expect(
  page.frameLocator('iframe[name=late]').locator('#name'),
).toHaveText('Late')
expect(Boolean(page.frame('late'))).toBe(true)
const target = `${origin}/child.html?name=MovedAgain`
await page.evaluate((target) => {
  setTimeout(() => {
    document.querySelector('iframe[name=cross]').src = target
  }, 200)
}, target)
const waited = await cross.waitForFunction(
  () => document.querySelector('#name')?.textContent === 'MovedAgain',
)
expect(await waited.jsonValue()).toBe(true)
await page.evaluate(() => {
  setTimeout(() => document.querySelector('iframe[name=same]').remove(), 200)
})
let removed = false
try {
  await same.waitForURL('**/never', { timeout: 2000 })
} catch {
  removed = true
}
expect(removed).toBe(true)
expect(same.isDetached()).toBe(true)
for (const host of ['localhost', '127.0.0.1']) {
  await page.goto(`${origin}/child.html`)
  const moved = `http://${host}:${new URL(origin).port}/child.html?name=AfterEvaluate`
  let rejected = false
  try {
    await page.evaluate((url) => {
      setTimeout(() => (location.href = url), 200)
      return new Promise(() => {})
    }, moved)
  } catch {
    rejected = true
  }
  expect(rejected).toBe(true)
  await page.waitForURL(moved)
  expect(await page.locator('#name').textContent()).toBe('AfterEvaluate')
}
