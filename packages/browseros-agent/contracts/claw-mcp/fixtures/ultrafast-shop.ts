#!/usr/bin/env bun
/**
 * A small shop for ultrafast skill tests. It behaves like a real store page (results render
 * after load, Add to cart updates a header count over fetch) and can be switched into the
 * situations a saved flow must survive: a changed layout before or after the write, a slow
 * server, or a captcha wall. State is in memory; `/admin/*` reads and switches it.
 *
 *   bun contracts/claw-mcp/fixtures/ultrafast-shop.ts [--port 14455]
 */

export type ShopLayout = 'v1' | 'v2' | 'v3'

export interface ShopState {
  cart: string[]
  layout: ShopLayout
  slowMs: number
  captcha: boolean
}

export interface UltrafastShop {
  origin: string
  url(path: string): string
  state: ShopState
  reset(): void
  stop(): Promise<void>
}

interface Product {
  id: string
  brand: string
  title: string
  price: string
  sponsored?: boolean
}

const PRODUCTS: Product[] = [
  {
    id: 'oralb-ad',
    brand: 'Oral-B',
    title: 'Sponsored Ad - Oral-B Pro Expert Toothpaste, 200 g',
    price: '₹199',
    sponsored: true,
  },
  {
    id: 'sensodyne',
    brand: 'Sensodyne',
    title: 'Sensodyne Fresh Mint Sensitive Toothpaste, 450 g',
    price: '₹463',
  },
  {
    id: 'colgate',
    brand: 'Colgate',
    title: 'Colgate Strong Teeth Anticavity Toothpaste, 500 g',
    price: '₹245',
  },
  {
    id: 'dove',
    brand: 'Dove',
    title: 'Dove Cream Beauty Bathing Soap, 125 g (pack of 4)',
    price: '₹259',
  },
  {
    id: 'boat',
    brand: 'boAt',
    title: 'boAt Rockerz 450 Bluetooth On-Ear Headphones',
    price: '₹1,499',
  },
]

function freshState(): ShopState {
  return { cart: [], layout: 'v1', slowMs: 0, captcha: false }
}

function escapeHtml(text: string): string {
  return text
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
}

function cartLabel(state: ShopState): string {
  const n = state.cart.length
  return state.layout === 'v3'
    ? `Basket: ${n}`
    : `${n} ${n === 1 ? 'item' : 'items'} in cart`
}

function search(query: string): Product[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  if (words.length === 0) return []
  const hits = PRODUCTS.filter((product) => {
    const haystack = `${product.brand} ${product.title}`.toLowerCase()
    return words.some((word) => word.length > 2 && haystack.includes(word))
  })
  // Like a real store, a sponsored card leads any non-empty result list.
  const ad = PRODUCTS.find((product) => product.sponsored)
  return hits.length && ad && !hits.includes(ad) ? [ad, ...hits] : hits
}

function page(title: string, state: ShopState, body: string): string {
  return `<!doctype html><html><head><meta charset="utf-8"><title>${escapeHtml(title)}</title>
<style>body{font:15px system-ui;margin:24px}article{border:1px solid #ccc;padding:12px;margin:8px 0;max-width:520px}header{margin-bottom:16px}</style>
</head><body><header><a href="/">Ultrafast Shop</a> · <a href="/cart" id="cart-count">${escapeHtml(cartLabel(state))}</a></header>${body}</body></html>`
}

function resultsPage(state: ShopState, query: string): string {
  const hits = search(query)
  const addLabel = state.layout === 'v2' ? 'Add to basket' : 'Add to cart'
  const cards = hits.map((product) => ({
    id: product.id,
    brand: state.layout === 'v2' ? null : product.brand,
    title: product.title,
    price: product.price,
  }))
  const body = `<h1>Results for “${escapeHtml(query)}”</h1>
<main data-results></main>
<script>
const cards = ${JSON.stringify(cards)};
const query = ${JSON.stringify(query)};
const addLabel = ${JSON.stringify(addLabel)};
setTimeout(() => {
  const main = document.querySelector('[data-results]');
  if (cards.length === 0) {
    main.innerHTML = '<p>No results for ' + query.replace(/[<>&]/g, '') + '</p>';
    return;
  }
  for (const card of cards) {
    const article = document.createElement('article');
    article.setAttribute('data-result', card.id);
    if (card.brand) {
      const h2 = document.createElement('h2');
      h2.textContent = card.brand;
      article.appendChild(h2);
    }
    const link = document.createElement('a');
    link.href = '/p/' + card.id;
    link.textContent = card.title;
    article.appendChild(link);
    const price = document.createElement('a');
    price.href = '/p/' + card.id;
    price.textContent = card.price;
    article.appendChild(price);
    const button = document.createElement('button');
    button.textContent = addLabel;
    button.addEventListener('click', async () => {
      button.disabled = true;
      const response = await fetch('/cart/add', { method: 'POST', body: card.id });
      const { label } = await response.json();
      setTimeout(() => {
        document.getElementById('cart-count').textContent = label;
        button.disabled = false;
      }, 200);
    });
    article.appendChild(button);
    main.appendChild(article);
  }
}, 300);
</script>`
  return page(`Results for ${query}`, state, body)
}

function cartPage(state: ShopState): string {
  const rows = state.cart
    .map((id, index) => {
      const product = PRODUCTS.find((candidate) => candidate.id === id)
      return `<li>${escapeHtml(product?.title ?? id)} <form method="post" action="/cart/remove" style="display:inline"><input type="hidden" name="index" value="${index}"><button>Remove</button></form></li>`
    })
    .join('')
  return page(
    'Cart',
    state,
    `<h1>Your cart</h1><ul>${rows || '<li>Your cart is empty</li>'}</ul>`,
  )
}

const html = (body: string, status = 200) =>
  new Response(body, {
    status,
    headers: { 'content-type': 'text/html; charset=utf-8' },
  })
const json = (value: unknown) => Response.json(value)

function handleAdmin(
  path: string,
  url: URL,
  method: string,
  state: ShopState,
): ShopState {
  if (path === '/admin/state' || method !== 'POST') return state
  if (path === '/admin/reset') return freshState()
  if (path === '/admin/layout') {
    const v = url.searchParams.get('v')
    if (v === 'v1' || v === 'v2' || v === 'v3') state.layout = v
  }
  if (path === '/admin/slow') {
    state.slowMs = Number(url.searchParams.get('ms') ?? 0) || 0
  }
  if (path === '/admin/captcha') {
    state.captcha = url.searchParams.get('on') === '1'
  }
  return state
}

const HOME =
  '<h1>Ultrafast Shop</h1><form action="/s"><input name="k" aria-label="Search"><button>Search</button></form>'
const CAPTCHA =
  '<h1>Verify you are human</h1><p>Tick the box to continue.</p><input type="checkbox" aria-label="I am not a robot">'

async function handleShop(
  request: Request,
  url: URL,
  state: ShopState,
): Promise<Response> {
  const path = url.pathname
  if (path === '/') return html(page('Ultrafast Shop', state, HOME))
  if (path === '/s') {
    if (state.captcha) {
      return html(page('Verify you are human', state, CAPTCHA))
    }
    return html(resultsPage(state, url.searchParams.get('k') ?? ''))
  }
  if (path === '/cart') return html(cartPage(state))
  if (path === '/cart/add' && request.method === 'POST') {
    const id = (await request.text()).trim()
    if (PRODUCTS.some((product) => product.id === id)) state.cart.push(id)
    return json({ count: state.cart.length, label: cartLabel(state) })
  }
  if (path === '/cart/remove' && request.method === 'POST') {
    const index = Number((await request.formData()).get('index'))
    if (Number.isInteger(index)) state.cart.splice(index, 1)
    return Response.redirect(`${url.origin}/cart`, 303)
  }
  const product = path.startsWith('/p/')
    ? PRODUCTS.find((candidate) => candidate.id === path.slice(3))
    : undefined
  if (!product) return html('not found', 404)
  return html(
    page(
      product.title,
      state,
      `<h1>${escapeHtml(product.title)}</h1><p>${escapeHtml(product.price)}</p>`,
    ),
  )
}

export async function startUltrafastShop(
  options: { port?: number } = {},
): Promise<UltrafastShop> {
  let state = freshState()
  const server = Bun.serve({
    port: options.port ?? 0,
    hostname: '127.0.0.1',
    async fetch(request) {
      const url = new URL(request.url)
      if (url.pathname.startsWith('/admin/')) {
        state = handleAdmin(url.pathname, url, request.method, state)
        return json(state)
      }
      if (state.slowMs > 0) await Bun.sleep(state.slowMs)
      return handleShop(request, url, state)
    },
  })
  const origin = `http://127.0.0.1:${server.port}`
  return {
    origin,
    url: (path: string) => `${origin}${path}`,
    get state() {
      return state
    },
    reset() {
      state = freshState()
    },
    async stop() {
      await server.stop(true)
    },
  }
}

if (import.meta.main) {
  const flag = process.argv.indexOf('--port')
  const port = flag > -1 ? Number(process.argv[flag + 1]) : 14455
  const shop = await startUltrafastShop({ port })
  console.log(`Ultrafast shop: ${shop.origin}`)
  console.log(`  search   ${shop.url('/s?k=sensodyne')}`)
  console.log(`  state    curl -s ${shop.url('/admin/state')}`)
  console.log(
    `  switch   curl -XPOST '${shop.url('/admin/layout?v=v2')}'  (v1 | v2 | v3), /admin/slow?ms=27000, /admin/captcha?on=1, /admin/reset`,
  )
}
