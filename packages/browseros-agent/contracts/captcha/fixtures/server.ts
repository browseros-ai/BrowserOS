import { FIXTURE_PAGES } from './catalog'

export interface CaptchaFixtureServer {
  origin: string
  url(path: string): string
  stop(): Promise<void>
}
type Vendor = 'recaptcha_v2' | 'turnstile' | 'hcaptcha'
const verification = {
  recaptcha_v2: {
    url: 'https://www.google.com/recaptcha/api/siteverify',
    field: 'g-recaptcha-response',
    secret: '6LeIxAcTAAAAAGG-vFI1TnRWxMZNFuojJ4WifJWe',
  },
  turnstile: {
    url: 'https://challenges.cloudflare.com/turnstile/v0/siteverify',
    field: 'cf-turnstile-response',
    secret: '1x0000000000000000000000000000000AA',
  },
  hcaptcha: {
    url: 'https://api.hcaptcha.com/siteverify',
    field: 'h-captcha-response',
    secret: '0x0000000000000000000000000000000000000000',
  },
}

function verdict(accepted: boolean, codes: string[] = []): Response {
  const text = accepted ? 'ACCEPTED' : `REJECTED: ${codes.join(', ')}`
  return new Response(
    `<!doctype html><html lang="en" data-captcha-verdict="${accepted ? 'accepted' : 'rejected'}"><head><meta charset="utf-8"><title>CAPTCHA verdict</title></head><body>${Bun.escapeHTML(text)}</body></html>`,
    {
      headers: { 'content-type': 'text/html; charset=utf-8' },
    },
  )
}

export async function startCaptchaFixtureServer(
  options: {
    hostname?: string
    publicHost?: string
    port?: number
    verifyUrls?: Partial<Record<Vendor, string>>
  } = {},
): Promise<CaptchaFixtureServer> {
  async function handle(request: Request): Promise<Response> {
    const { pathname } = new URL(request.url)
    if (request.method === 'GET') {
      if (pathname === '/healthz') return new Response('ok')
      if (FIXTURE_PAGES.some((page) => page.path === pathname)) {
        return new Response(Bun.file(`${import.meta.dir}/pages${pathname}`), {
          headers: { 'content-type': 'text/html; charset=utf-8' },
        })
      }
    }
    const page = FIXTURE_PAGES.find((page) => pathname === `/submit/${page.id}`)
    if (request.method !== 'POST' || !page)
      return new Response('not found', { status: 404 })
    if (page.family === 'none') return verdict(true)
    const config = verification[page.family]
    try {
      const token = (await request.formData()).get(config.field)
      if (typeof token !== 'string' || !token.trim())
        return verdict(false, ['missing-input-response'])
      const secret =
        page.id === 'turnstile-block'
          ? '2x0000000000000000000000000000000AA'
          : config.secret
      const response = await fetch(
        options.verifyUrls?.[page.family] ?? config.url,
        {
          method: 'POST',
          body: new URLSearchParams({ secret, response: token }),
          signal: AbortSignal.timeout(10_000),
        },
      )
      if (!response.ok) return verdict(false, ['verification-unavailable'])
      const result = (await response.json()) as {
        success?: unknown
        'error-codes'?: unknown
      }
      const codes = Array.isArray(result['error-codes'])
        ? result['error-codes'].filter(
            (code): code is string => typeof code === 'string',
          )
        : []
      return verdict(
        result.success === true,
        codes.length ? codes : ['verification-failed'],
      )
    } catch {
      return verdict(false, ['verification-unavailable'])
    }
  }
  let lastError: unknown
  for (let attempt = 0; attempt < 20; attempt++) {
    try {
      const server = Bun.serve({
        hostname: options.hostname ?? '127.0.0.1',
        port: options.port ?? 10101 + Math.floor(Math.random() * 10102),
        fetch: handle,
      })
      const host = options.publicHost ?? '127.0.0.1'
      const origin = `http://${host.includes(':') && !host.startsWith('[') ? `[${host}]` : host}:${server.port}`
      return {
        origin,
        url: (path) => `${origin}${path.startsWith('/') ? path : `/${path}`}`,
        stop: async () => {
          await server.stop(true)
        },
      }
    } catch (error) {
      if (options.port !== undefined) throw error
      lastError = error
    }
  }
  throw new Error(`Could not bind a CAPTCHA fixture port: ${lastError}`)
}
