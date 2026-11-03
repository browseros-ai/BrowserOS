/**
 * Step sources for the ultrafast shop flows used by the contract cases: open the search,
 * find the organic card, click Add to cart (the commit), confirm the cart count.
 */

export const SHOP_PROSE = [
  'Open the shop search for the query.',
  'Find the first organic result whose brand or title matches, and note the cart count.',
  'Click its Add to cart button.',
  'Confirm the cart count went up by one.',
]

export const SHOP_TYPES = {
  inputs: { query: 'string', match: 'string' },
  output: { added: 'string', price: 'string', cartCount: 'number' },
}

export const SHOP_EXAMPLES = [
  { query: 'sensodyne toothpaste', match: 'sensodyne' },
  { query: 'colgate toothpaste', match: 'colgate' },
  { query: 'dove soap', match: 'dove' },
]

export function shopSteps(origin: string, addLabel = 'Add to cart') {
  const open = `async ({ browser, inputs, state }) => {
  state.page = await browser.open('${origin}/s?k=' + encodeURIComponent(inputs.query));
}`
  const find = `async ({ inputs, state, handBack }) => {
  const parse = (text) => {
    const cards = [];
    let card = null;
    for (const line of text.split('\\n')) {
      const t = line.trim();
      if (t === '- article') { card = { brand: null, title: null, price: null, addRef: null }; cards.push(card); continue; }
      if (!card) continue;
      let m;
      if ((m = t.match(/^- heading "([^"]+)" \\[level=2\\]/))) card.brand = m[1];
      else if ((m = t.match(/^- link "(₹[^"]+)"/))) card.price = m[1];
      else if ((m = t.match(/^- link "([^"]{12,})" \\[ref=(e\\d+)\\]/))) card.title = m[1];
      else if ((m = t.match(/^- button "${addLabel}" \\[ref=(e\\d+)\\]/))) card.addRef = m[1];
    }
    return cards;
  };
  const ready = await state.page.waitForSelector('[data-result], [data-results] p', { timeout: 6000 });
  if (!ready.matched) handBack('wait_timeout', { expected: 'search results' });
  const snap = await state.page.snapshot();
  const cards = parse(snap.text);
  const want = inputs.match.toLowerCase();
  state.card = cards.find((c) => c.addRef && c.title && !c.title.startsWith('Sponsored') &&
    ((c.brand || '').toLowerCase().includes(want) || c.title.toLowerCase().includes(want))) || null;
  if (!state.card) handBack('not_found', { query: inputs.query, cardsSeen: cards.length });
  const count = snap.text.match(/link "(\\d+) items? in cart"/);
  state.before = count ? Number(count[1]) : 0;
  return { title: state.card.title, price: state.card.price };
}`
  const click = `async ({ state }) => {
  await state.page.click(state.card.addRef);
}`
  const confirm = `async ({ state, handBack }) => {
  const want = state.before + 1;
  const label = want + (want === 1 ? ' item' : ' items') + ' in cart';
  const ok = await state.page.waitForText(label, { timeout: 5000 });
  if (!ok.matched) handBack('wait_timeout', { expected: label });
  return { added: state.card.title, price: state.card.price, cartCount: want };
}`
  return [
    { step: 1, fn: open },
    { step: 2, fn: find },
    { step: 3, fn: click, commit: true },
    { step: 4, fn: confirm },
  ]
}
