import { beforeAll, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

type MockButtonProps = ComponentProps<'button'> & {
  variant?: string
  size?: string
}

mock.module('@/modules/analytics/events', () => ({
  AnalyticsEvent: {
    ProductHuntBannerShown: 'product_hunt_banner_shown',
    ProductHuntBannerClicked: 'product_hunt_banner_clicked',
    ProductHuntBannerDismissed: 'product_hunt_banner_dismissed',
  },
  track: () => {},
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

mock.module('@/components/ui/svgs/productHuntIcon', () => ({
  ProductHuntIcon: () => createElement('svg'),
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

  it('renders nothing outside the launch window', () => {
    const html = renderToStaticMarkup(createElement(ProductHuntBanner))

    expect(html).toBe('')
  })
})
