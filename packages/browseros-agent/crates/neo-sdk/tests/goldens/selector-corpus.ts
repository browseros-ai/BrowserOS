// Golden corpus of locator chain -> Playwright selector string, for Neo's locator builder to match.
//
// The selector builders and escapers are Playwright's own (imported from the local checkout). Only the
// chain composition in packages/playwright-core/src/client/{page,frame,locator}.ts is mirrored here,
// because those classes need a live protocol connection to construct. Each chain is a JS expression an
// agent would write; it is evaluated against the mirror, so the recorded chain text is exactly what
// produced its selector. When it was captured, every chain also matched real playwright-core 1.64.1
// (94/94; README.md).
//
// Run: bun selector-corpus.ts   (writes selector-goldens.json next to this file)

import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';

import {
  getByAltTextSelector,
  getByLabelSelector,
  getByPlaceholderSelector,
  getByRoleSelector,
  getByTestIdSelector,
  getByTextSelector,
  getByTitleSelector,
  type ByRoleOptions,
} from '/Users/shadowfax/code/oss/playwright/packages/isomorphic/locatorUtils.ts';
import { escapeForTextSelector } from '/Users/shadowfax/code/oss/playwright/packages/isomorphic/stringUtils.ts';

const PLAYWRIGHT_CHECKOUT = '/Users/shadowfax/code/oss/playwright';
const EXPECTED_COMMIT = '26137face';

const head = execFileSync('git', ['-C', PLAYWRIGHT_CHECKOUT, 'rev-parse', '--short=9', 'HEAD'], { encoding: 'utf8' }).trim();
if (head !== EXPECTED_COMMIT)
  throw new Error(`Playwright checkout is at ${head}, goldens are pinned to ${EXPECTED_COMMIT}`);

type LocatorOptions = {
  hasText?: string | RegExp;
  hasNotText?: string | RegExp;
  has?: Locator;
  hasNot?: Locator;
  visible?: boolean;
};

// locator.ts:528. setTestIdAttribute() is never called, so the default holds.
const testIdAttributeName = 'data-testid';

// The getBy* bodies are identical on Frame (frame.ts:370-396) and Locator (locator.ts:195-221): each wraps a
// locatorUtils builder in this.locator(). Page's getBy* (page.ts:773-802) forward to mainFrame().
// The mirror has a single frame, so the "must belong to the same frame" guards (locator.ts:67,74,186,268,274)
// can never fire and are left out.
abstract class Scope {
  abstract locator(selector: string, options?: LocatorOptions): Locator;

  getByTestId(testId: string | RegExp) {
    return this.locator(getByTestIdSelector(testIdAttributeName, testId));
  }

  getByAltText(text: string | RegExp, options?: { exact?: boolean }) {
    return this.locator(getByAltTextSelector(text, options));
  }

  getByLabel(text: string | RegExp, options?: { exact?: boolean }) {
    return this.locator(getByLabelSelector(text, options));
  }

  getByPlaceholder(text: string | RegExp, options?: { exact?: boolean }) {
    return this.locator(getByPlaceholderSelector(text, options));
  }

  getByText(text: string | RegExp, options?: { exact?: boolean }) {
    return this.locator(getByTextSelector(text, options));
  }

  getByTitle(text: string | RegExp, options?: { exact?: boolean }) {
    return this.locator(getByTitleSelector(text, options));
  }

  getByRole(role: string, options: ByRoleOptions = {}) {
    return this.locator(getByRoleSelector(role, options));
  }
}

class Page extends Scope {
  // page.ts:769 -> frame.ts:366: the selector is used as-is, options applied by the constructor.
  locator(selector: string, options?: LocatorOptions) {
    return new Locator(selector, options);
  }
}

class Locator extends Scope {
  readonly selector: string;

  // locator.ts:55-80. Options append in this fixed order regardless of object key order.
  // hasText/hasNotText are truthiness checks, so '' adds nothing; visible checks !== undefined.
  constructor(selector: string, options?: LocatorOptions) {
    super();
    let s = selector;
    if (options?.hasText)
      s += ` >> internal:has-text=${escapeForTextSelector(options.hasText, false)}`;
    if (options?.hasNotText)
      s += ` >> internal:has-not-text=${escapeForTextSelector(options.hasNotText, false)}`;
    if (options?.has)
      s += ` >> internal:has=` + JSON.stringify(options.has.selector);
    if (options?.hasNot)
      s += ` >> internal:has-not=` + JSON.stringify(options.hasNot.selector);
    if (options?.visible !== undefined)
      s += ` >> visible=${options.visible ? 'true' : 'false'}`;
    this.selector = s;
  }

  // locator.ts:183-189. A Locator argument is chained through internal:chain, not concatenated.
  locator(selectorOrLocator: string | Locator, options?: Omit<LocatorOptions, 'visible'>): Locator {
    if (typeof selectorOrLocator === 'string')
      return new Locator(this.selector + ' >> ' + selectorOrLocator, options);
    return new Locator(this.selector + ' >> internal:chain=' + JSON.stringify(selectorOrLocator.selector), options);
  }

  // locator.ts:227-229
  filter(options?: LocatorOptions) {
    return new Locator(this.selector, options);
  }

  // locator.ts:231-233
  visible() {
    return new Locator(this.selector, { visible: true });
  }

  // locator.ts:255-265
  first() {
    return new Locator(this.selector + ' >> nth=0');
  }

  last() {
    return new Locator(this.selector + ` >> nth=-1`);
  }

  nth(index: number) {
    return new Locator(this.selector + ` >> nth=${index}`);
  }

  // locator.ts:267-277
  and(locator: Locator) {
    return new Locator(this.selector + ` >> internal:and=` + JSON.stringify(locator.selector));
  }

  or(locator: Locator) {
    return new Locator(this.selector + ` >> internal:or=` + JSON.stringify(locator.selector));
  }
}

// Each entry is source text. String.raw keeps backslashes exactly as an agent would type them.
const CHAINS: string[] = [
  // getByRole: options and their fixed output order (checked, disabled, selected, expanded, include-hidden,
  // level, name, description, pressed; locatorUtils.ts:69-90).
  `page.getByRole('button')`,
  `page.getByRole('button', { name: 'Submit' })`,
  `page.getByRole('button', { name: 'Submit', exact: true })`,
  `page.getByRole('button', { name: 'Submit', exact: false })`,
  `page.getByRole('button', { name: /submit/i })`,
  `page.getByRole('button', { name: /submit/, exact: true })`,
  `page.getByRole('button', { name: '' })`,
  `page.getByRole('heading', { level: 2 })`,
  `page.getByRole('heading', { name: 'Title', level: 1 })`,
  `page.getByRole('checkbox', { checked: true })`,
  `page.getByRole('checkbox', { checked: false })`,
  `page.getByRole('checkbox', { checked: 'mixed' })`,
  `page.getByRole('button', { disabled: true })`,
  `page.getByRole('button', { expanded: false })`,
  `page.getByRole('button', { pressed: true })`,
  `page.getByRole('button', { pressed: 'mixed' })`,
  `page.getByRole('option', { selected: true })`,
  `page.getByRole('button', { includeHidden: true })`,
  `page.getByRole('button', { description: 'Saves the form' })`,
  `page.getByRole('button', { pressed: false, name: 'Go', level: 3, includeHidden: false, expanded: true, selected: false, disabled: true, checked: true, exact: true })`,

  // Quotes, backslashes, control characters and unicode in string names.
  `page.getByRole('button', { name: 'Say "hi"' })`,
  `page.getByRole('link', { name: "It's here" })`,
  String.raw`page.getByRole('button', { name: 'C:\\temp\\new' })`,
  String.raw`page.getByRole('button', { name: 'tab\there\nnewline' })`,
  `page.getByRole('button', { name: 'Café ✓ 日本語 🎉' })`,

  // RegExp names: flags kept, quotes and >> escaped unless the u/v flag is set (stringUtils.ts:101-110).
  `page.getByRole('button', { name: /say "hi"/gi })`,
  `page.getByRole('button', { name: /it's/ })`,
  String.raw`page.getByRole('button', { name: /a\"b/ })`,
  String.raw`page.getByRole('button', { name: /a\\"b/ })`,
  `page.getByRole('button', { name: /café "x"/u })`,
  `page.getByRole('button', { name: /a >> b/ })`,

  // getByText / getByLabel / getByPlaceholder / getByAltText / getByTitle, string and RegExp.
  `page.getByText('Welcome')`,
  `page.getByText('Welcome', { exact: true })`,
  `page.getByText(/welcome/i)`,
  `page.getByText(/^Hello$/m, { exact: true })`,
  String.raw`page.getByText('line1\nline2 "q" \\ end')`,
  String.raw`page.getByText(/\d+ items? "left"/s)`,
  `page.getByText(/a >> b/)`,
  `page.getByText(/x/v)`,
  `page.getByText('emoji 🎉 ünïcödé')`,
  `page.getByLabel('Email')`,
  `page.getByLabel('Email', { exact: true })`,
  `page.getByLabel(/e-?mail/i)`,
  `page.getByPlaceholder('Search')`,
  `page.getByPlaceholder('Search', { exact: true })`,
  `page.getByPlaceholder(/^search/)`,
  `page.getByAltText('Logo')`,
  `page.getByAltText(/logo/i, { exact: true })`,
  `page.getByTitle('Close')`,
  String.raw`page.getByTitle('a "b" \\ c', { exact: true })`,
  `page.getByTitle(/clo"se/)`,

  // getByTestId is always exact for strings.
  `page.getByTestId('submit-btn')`,
  `page.getByTestId('say "x"')`,
  String.raw`page.getByTestId(/submit-\d+/)`,

  // css, text, xpath and chained selectors via locator(string).
  `page.locator('button.primary')`,
  `page.locator('text=Submit')`,
  `page.locator('text="Submit"')`,
  `page.locator('xpath=//button[@id="go"]')`,
  `page.locator('div >> span')`,
  `page.locator('#form').locator('input[name="email"]')`,

  // Constructor options on page.locator() and locator().locator(); output order is fixed (locator.ts:59-80).
  `page.locator('div', { hasText: 'Hello' })`,
  `page.locator('div', { hasText: /hello "w"/i })`,
  `page.locator('div', { hasNotText: 'Bye' })`,
  `page.locator('div', { hasText: '' })`,
  `page.locator('li', { has: page.getByRole('button', { name: 'Delete' }) })`,
  `page.locator('li', { hasNot: page.locator('.done') })`,
  `page.locator('button', { visible: true })`,
  `page.locator('button', { visible: false })`,
  `page.locator('div', { visible: true, hasNot: page.locator('em'), has: page.locator('span'), hasNotText: 'b', hasText: 'a' })`,
  `page.locator('#a').locator('span', { hasText: 'x' })`,

  // Nested chains, including a Locator passed to locator() (internal:chain).
  `page.locator('form').getByLabel('Password')`,
  `page.getByRole('listitem').filter({ hasText: 'Apple' }).getByRole('button', { name: 'Buy' })`,
  `page.getByTestId('list').locator(page.getByRole('listitem'))`,
  `page.getByTestId('list').locator(page.getByRole('listitem', { name: 'x"y' }), { hasText: 'z' })`,

  // filter()
  `page.getByRole('listitem').filter({ hasText: 'Apple' })`,
  `page.getByRole('listitem').filter({ hasText: /apple/i })`,
  `page.getByRole('listitem').filter({ hasNotText: 'Out of stock' })`,
  `page.getByRole('listitem').filter({ hasNotText: /sold out/i })`,
  `page.getByRole('listitem').filter({ has: page.getByRole('button', { name: 'Add' }) })`,
  `page.getByRole('listitem').filter({ hasNot: page.getByText('Sold') })`,
  `page.getByRole('listitem').filter({ visible: true })`,
  `page.getByRole('listitem').filter()`,
  `page.getByRole('listitem').filter({ hasText: 'A' }).filter({ hasNotText: 'B' })`,
  `page.locator('li').filter({ has: page.locator('span').filter({ hasText: 'x"y' }) })`,
  `page.getByRole('button').visible()`,

  // nth / first / last
  `page.getByRole('button').first()`,
  `page.getByRole('button').last()`,
  `page.getByRole('button').nth(2)`,
  `page.getByRole('button').nth(-1)`,
  `page.locator('li').nth(0).getByText('x')`,

  // and / or
  `page.getByRole('button').and(page.getByTitle('Subscribe'))`,
  `page.getByRole('button', { name: 'New' }).or(page.getByText('Confirm'))`,
  `page.locator('a').or(page.locator('b')).first()`,
  `page.getByRole('button').and(page.locator('.x').or(page.locator('.y')))`,
];

const page = new Page();
const goldens = CHAINS.map(chain => {
  const locator = new Function('page', `return ${chain};`)(page);
  if (!(locator instanceof Locator))
    throw new Error(`Chain did not produce a Locator: ${chain}`);
  return { chain, selector: locator.selector };
});

const duplicates = goldens.filter((g, i) => goldens.findIndex(o => o.chain === g.chain) !== i);
if (duplicates.length)
  throw new Error(`Duplicate chains: ${duplicates.map(d => d.chain).join(', ')}`);

const out = join(import.meta.dir, 'selector-goldens.json');
writeFileSync(out, JSON.stringify(goldens, null, 2) + '\n');
console.log(`playwright ${head}: wrote ${goldens.length} chains to ${out}`);
