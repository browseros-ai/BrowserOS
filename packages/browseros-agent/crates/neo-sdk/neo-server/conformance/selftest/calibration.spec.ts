import { expect, test } from '@playwright/test'

test('must-pass', async ({ page }) => {
  await page.setContent('<p>same behavior</p>')
  await expect(page.locator('p')).toHaveText('same behavior')
})
test('oracle-fails', async ({ page }) => {
  await page.setContent('<title>actual title</title>')
  expect(await page.title()).toBe('not the title')
})
test('neo-refuses', async ({ page }) => {
  await page.route('**/*', (route) => route.continue())
})
test('caught-refusal', async ({ page }) => {
  try {
    await page.evaluateHandle(() => {
      throw new Error('caught diagnostic calibration')
    })
    throw new Error('evaluation unexpectedly succeeded')
  } catch (error) {
    expect(error.message).toContain('caught diagnostic calibration')
  }
})
test('neo-differs', async ({ page }) => {
  await page.setContent('<input type="password" aria-label="password">')
  await page.evaluate(() => {
    document.querySelector('input').value = String.fromCharCode(120).repeat(13)
  })
  expect((await page.locator('input').inputValue()).length).toBe(13)
})
test('count-differs', async ({ page }) => {
  expect(await page.url()).toBe('about:blank')
  if (typeof __neo !== 'undefined') expect(true).toBe(true)
})
test('runtime-globals', async ({ page }) => {
  expect(atob(btoa('hello'))).toBe('hello')
  expect(new URL('/path?q=1', 'https://example.com').href).toBe(
    'https://example.com/path?q=1',
  )
  expect(new URLSearchParams('a=one+two').get('a')).toBe('one two')
  expect(await page.url()).toBe('about:blank')
})
test('transport-timeout', async ({ page }) => {
  await page.setContent('<p>before timeout</p>')
  await new Promise((resolve) => setTimeout(resolve, 2000))
})

test('viewport', async ({ page, server }, info) => {
  const measure = () =>
    page.evaluate(() => ({
      width: innerWidth,
      height: innerHeight,
      dpr: devicePixelRatio,
    }))
  info.measurements.viewport = await measure()
  expect(info.measurements.viewport.width).toBeGreaterThan(0)
  await page.goto(`${server.CROSS_PROCESS_PREFIX}/empty.html`)
  info.measurements.viewportAfterNavigation = await measure()
  expect(info.measurements.viewportAfterNavigation.width).toBeGreaterThan(0)
})
