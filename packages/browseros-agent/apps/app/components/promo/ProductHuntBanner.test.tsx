import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

type MockButtonProps = ComponentProps<'button'> & {
  variant?: string
  size?: string
}

mock.module('@/assets/producthunt.svg', () => ({
  default: 'producthunt.svg',
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

let ProductHuntBanner: FC
let ProductHuntBannerCard: FC<{
  onOpen: () => void
  onDismiss: () => void
}>

beforeAll(async () => {
  const bannerModule = await import('./ProductHuntBanner')
  ProductHuntBanner = bannerModule.ProductHuntBanner
  ProductHuntBannerCard = bannerModule.ProductHuntBannerCard
})

describe('ProductHuntBanner', () => {
  it('renders the launch copy and Product Hunt CTA', () => {
    const html = renderToStaticMarkup(
      createElement(ProductHuntBannerCard, {
        onOpen: () => {},
        onDismiss: () => {},
      }),
    )

    expect(html).toContain('live on Product Hunt')
    expect(html).toContain('Check out our launch')
    expect(html).toContain('Product Hunt')
  })

  it('renders nothing until persisted visibility and the launch window resolve', () => {
    const html = renderToStaticMarkup(createElement(ProductHuntBanner))

    expect(html).toBe('')
  })
})
