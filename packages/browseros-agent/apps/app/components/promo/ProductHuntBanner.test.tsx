import { beforeAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

type MockButtonProps = ComponentProps<'button'> & {
  variant?: string
  size?: string
}

mock.module('@/assets/producthunt.svg', () => ({
  default: 'producthunt.svg',
}))

mock.module('react-router', () => ({
  useNavigate: () => () => {},
}))

mock.module('@/lib/metrics/track', () => ({
  track: () => {},
}))

mock.module('@/lib/sentry/sentry', () => ({
  sentry: {
    captureException: () => {},
  },
}))

mock.module('@/lib/constants/analyticsEvents', () => ({
  PRODUCT_HUNT_BANNER_SHOWN_EVENT: 'ui.product_hunt_banner.shown',
  PRODUCT_HUNT_BANNER_CLICKED_EVENT: 'ui.product_hunt_banner.clicked',
  PRODUCT_HUNT_BANNER_DISMISSED_EVENT: 'ui.product_hunt_banner.dismissed',
}))

mock.module('@/components/ui/button', () => ({
  Button: ({
    children,
    variant: _variant,
    size: _size,
    ...props
  }: MockButtonProps) =>
    createElement('button', { type: 'button', ...props }, children),
}))

mock.module('./product-hunt-banner.storage', () => ({
  productHuntBannerDismissedStorage: {
    getValue: async () => false,
    setValue: async () => {},
    watch: () => () => {},
  },
}))

/** Records the exact argument object handed to chrome.tabs.create. */
let tabsCreateCalls: Array<{ url?: string; active?: boolean }> = []

beforeEach(() => {
  tabsCreateCalls = []
})

;(globalThis as unknown as { chrome: unknown }).chrome = {
  tabs: {
    create: (options: { url?: string; active?: boolean }) => {
      tabsCreateCalls.push(options)
    },
  },
}

let ProductHuntBanner: FC
let ProductHuntBannerCard: FC<{
  onOpen: () => void
  onDismiss: () => void
}>
let openProductHuntInBackground: () => void
let openProductHuntFocused: () => void

beforeAll(async () => {
  const bannerModule = await import('./ProductHuntBanner')
  ProductHuntBanner = bannerModule.ProductHuntBanner
  ProductHuntBannerCard = bannerModule.ProductHuntBannerCard
  openProductHuntInBackground = bannerModule.openProductHuntInBackground
  openProductHuntFocused = bannerModule.openProductHuntFocused
})

describe('ProductHuntBanner', () => {
  it('renders the launch copy and Product Hunt CTA', () => {
    const html = renderToStaticMarkup(
      createElement(ProductHuntBannerCard, {
        onOpen: () => {},
        onDismiss: () => {},
      }),
    )

    expect(html).toContain('Upvote us on Product Hunt and get 100 credits')
    expect(html).toContain('Upvote us')
    expect(html).toContain('Product Hunt')
  })

  it('renders nothing until persisted visibility and the launch window resolve', () => {
    const html = renderToStaticMarkup(createElement(ProductHuntBanner))

    expect(html).toBe('')
  })
})

/**
 * These assert the whole argument object, not just that tabs.create was called.
 * The point is to pin `active`, so a call-count assertion would not have caught
 * the bug these exist to prevent - a later tidy-up dropping the flag as
 * redundant, silently handing the foreground back to Product Hunt.
 */
describe('opening Product Hunt', () => {
  it('opens in a background tab from the banner', () => {
    openProductHuntInBackground()

    expect(tabsCreateCalls).toEqual([
      { url: 'https://bit.ly/browseros-ph', active: false },
    ])
    // Spelled out separately: `false` is the whole point, and toEqual would
    // also pass if the key were merely absent.
    expect(tabsCreateCalls[0].active).toBe(false)
  })

  it('opens focused from the card button', () => {
    openProductHuntFocused()

    expect(tabsCreateCalls).toEqual([{ url: 'https://bit.ly/browseros-ph' }])
    // Omitting `active` is deliberate - Chrome defaults it to true, which is
    // what an explicit "take me there" click should do.
    expect(tabsCreateCalls[0].active).toBeUndefined()
  })

  it('keeps the two paths different', () => {
    openProductHuntInBackground()
    openProductHuntFocused()

    expect(tabsCreateCalls[0].active).toBe(false)
    expect(tabsCreateCalls[1].active).toBeUndefined()
  })
})
