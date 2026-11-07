/** Records frame selectors from the real pinned Playwright client on a throwaway browser. */
import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const playwright = process.env.NEO_PARITY_PLAYWRIGHT
const executablePath = process.env.NEO_E2E_BROWSER
if (!playwright || !executablePath)
  throw new Error('Set NEO_PARITY_PLAYWRIGHT and NEO_E2E_BROWSER')
const head = execFileSync('git', ['-C', playwright, 'rev-parse', 'HEAD'], { encoding: 'utf8' }).trim()
if (!head.startsWith('26137face'))
  throw new Error(`Expected Playwright 26137face, got ${head}`)

const { chromium } = await import(pathToFileURL(join(playwright, 'index.mjs')).href)
const browser = await chromium.launch({
  executablePath,
  headless: true,
  args: ['--disable-browseros-server'],
})
try {
  const page = await browser.newPage()
  const chains = [
    `page.frameLocator('#login').locator('input')`,
    `page.frameLocator('#login').locator(page.getByLabel('Email'))`,
    `page.frameLocator('#login').locator('li', { has: page.getByText('Item'), hasNot: page.getByText('Gone'), hasText: 'a', hasNotText: /b/i, visible: true })`,
    `page.frameLocator('#login').getByTestId('submit')`,
    String.raw`page.frameLocator('#login').getByTestId(/submit-\d+/)`,
    `page.frameLocator('#login').getByAltText('Logo', { exact: true })`,
    `page.frameLocator('#login').getByAltText(/logo/i)`,
    `page.frameLocator('#login').getByLabel('Email')`,
    `page.frameLocator('#login').getByLabel(/e-?mail/i, { exact: true })`,
    `page.frameLocator('#login').getByPlaceholder('Search', { exact: true })`,
    `page.frameLocator('#login').getByPlaceholder(/search/i)`,
    `page.frameLocator('#login').getByText('Welcome', { exact: true })`,
    `page.frameLocator('#login').getByText(/a >> b/)`,
    `page.frameLocator('#login').getByTitle('Close', { exact: true })`,
    `page.frameLocator('#login').getByTitle(/clo"se/u)`,
    `page.frameLocator('#login').getByRole('button', { name: 'Submit', exact: true })`,
    `page.frameLocator('#login').getByRole('checkbox', { checked: true, name: /agree/i })`,
    `page.frameLocator('#outer').frameLocator('#inner').getByLabel('Email')`,
    `page.frameLocator('#outer').frameLocator('#inner').first().getByText('Pay')`,
    `page.frameLocator('iframe').first().getByText('First')`,
    `page.frameLocator('iframe').last().getByText('Last')`,
    `page.frameLocator('iframe').nth(2).getByText('Third')`,
    `page.frameLocator('iframe').nth(-1).owner()`,
    `page.frameLocator('#outer').frameLocator('#inner').owner()`,
    `page.locator('#outer').frameLocator('#inner').getByLabel('Email')`,
    `page.locator('#outer').contentFrame().getByRole('button')`,
    `page.locator('#outer').contentFrame().frameLocator('#inner').getByRole('button')`,
    `page.locator('iframe').nth(1).contentFrame().getByText('Pay')`,
    `page.locator('iframe').contentFrame().owner()`,
    `page.mainFrame().frameLocator('#login').getByLabel('Email')`,
    `page.frameLocator().getByLabel('Email')`,
    `page.mainFrame().frameLocator().getByRole('button')`,
    `page.frameLocator().locator(page.getByText('Pay'))`,
    `page.frameLocator().frameLocator('#inner').getByText('Pay')`,
    `page.frameLocator().owner()`,
    `page.frameLocator('#login').getByText('Pay').filter({ hasText: /Pay/ }).nth(1)`,
    `page.frameLocator('#login').getByText('Pay').and(page.frameLocator('#login').getByRole('button'))`,
  ]
  const frameChains = [
    `page.frameLocator('#login')`,
    `page.frameLocator('#outer').frameLocator('#inner')`,
    `page.frameLocator('iframe').first()`,
    `page.frameLocator('iframe').last()`,
    `page.frameLocator('iframe').nth(2)`,
    `page.locator('#outer').frameLocator('#inner')`,
    `page.locator('#outer').contentFrame()`,
    `page.frameLocator()`,
  ]
  const evaluate = (chain) => new Function('page', `return ${chain}`)(page)
  const goldens = [
    ...chains.map((chain) => ({ chain, selector: evaluate(chain)._selector })),
    ...frameChains.map((chain) => ({ chain, kind: 'frameLocator', selector: evaluate(chain).owner()._selector })),
  ]
  const output = join(dirname(fileURLToPath(import.meta.url)), 'frame-selector-goldens.json')
  if (process.argv.includes('--check')) {
    if (JSON.stringify(JSON.parse(readFileSync(output, 'utf8'))) !== JSON.stringify(goldens))
      throw new Error(`${output} is stale`)
  } else {
    writeFileSync(output, `${JSON.stringify(goldens, null, 2)}\n`)
  }
  console.log(`Playwright ${head.slice(0, 9)}: ${goldens.length} frame selector goldens match`)
} finally {
  await browser.close()
}
