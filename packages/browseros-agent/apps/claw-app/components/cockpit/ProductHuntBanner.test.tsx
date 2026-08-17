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

const memoryStore: Record<string, string> = {}
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (key: string) => memoryStore[key] ?? null,
    setItem: (key: string, value: string) => {
      memoryStore[key] = value
    },
    removeItem: (key: string) => {
      delete memoryStore[key]
    },
  },
})

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

    expect(html).toContain('Live on Product Hunt.')
    expect(html).toContain('A vote or comment helps keep BrowserOS free.')
    expect(html).toContain('Support →')
    expect(html).toContain('Product Hunt')
  })

  it('renders nothing once dismissed', () => {
    memoryStore.productHuntBannerDismissed = 'true'
    try {
      const html = renderToStaticMarkup(createElement(ProductHuntBanner))
      expect(html).toBe('')
    } finally {
      delete memoryStore.productHuntBannerDismissed
    }
  })
})
