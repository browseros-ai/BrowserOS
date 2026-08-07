import { beforeAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import { type ComponentProps, createElement, type FC } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

type MockButtonProps = ComponentProps<'button'> & {
  variant?: string
  size?: string
}

type Submission = {
  mutate: () => void
  reset: () => void
  isPending: boolean
  isSuccess: boolean
  isError: boolean
  error: Error | null
}

const idleSubmission: Submission = {
  mutate: () => {},
  reset: () => {},
  isPending: false,
  isSuccess: false,
  isError: false,
  error: null,
}

let submission: Submission = idleSubmission

mock.module('@/assets/producthunt.svg', () => ({
  default: 'producthunt.svg',
}))

mock.module('@/components/promo/ProductHuntBanner', () => ({
  PRODUCT_HUNT_URL: 'https://bit.ly/browseros-ext',
}))

mock.module('sonner', () => ({
  toast: { error: () => {} },
}))

mock.module('@/lib/sentry/sentry', () => ({
  sentry: { captureException: () => {} },
}))

mock.module('./credit-request.hooks', () => ({
  DISCORD_HANDLE_MAX_LENGTH: 64,
  useSubmitCreditRequest: () => submission,
}))

mock.module('@/lib/constants/productUrls', () => ({
  discordUrl: 'https://discord.gg/browseros',
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

mock.module('@/components/ui/input', () => ({
  Input: (props: ComponentProps<'input'>) => createElement('input', props),
}))

mock.module('@/components/ui/label', () => ({
  Label: (props: ComponentProps<'label'>) => createElement('label', props),
}))

let CreditRequestCard: FC<{ browserosId?: string }>

const BROWSEROS_ID = 'c481ffe7-e00a-4412-8070-696a45f444a1'

beforeAll(async () => {
  CreditRequestCard = (await import('./CreditRequestCard')).CreditRequestCard
})

beforeEach(() => {
  submission = idleSubmission
})

function render(browserosId?: string) {
  return renderToStaticMarkup(createElement(CreditRequestCard, { browserosId }))
}

describe('CreditRequestCard', () => {
  it('shows the ID, the copy button and the handle form once the ID is known', () => {
    const html = render(BROWSEROS_ID)

    expect(html).toContain('Upvote us and get free credits')
    expect(html).toContain(BROWSEROS_ID)
    expect(html).toContain('font-mono')
    expect(html).toContain('Copy')
    expect(html).toContain('id="discord-handle"')
    expect(html).toContain('maxLength="64"')
    expect(html).toContain('https://discord.gg/browseros')
    expect(html).toContain('Upvote on Product Hunt')
  })

  for (const [label, browserosId] of [
    ['missing', undefined],
    ['empty', ''],
    ['whitespace only', '   '],
  ] as const) {
    it(`falls back to guidance instead of a copyable blank when the ID is ${label}`, () => {
      const html = render(browserosId)

      expect(html).toContain('available right now')
      expect(html).not.toContain('undefined')
      // Nothing to copy and nothing to submit: both controls must be absent,
      // not merely disabled, so an empty ID can never reach the clipboard.
      expect(html).not.toContain('Copy')
      expect(html).not.toContain('id="discord-handle"')
      expect(html).not.toContain('Submit')
      // The card still has to explain how to reach us.
      expect(html).toContain('https://discord.gg/browseros')
      expect(html).toContain('Upvote on Product Hunt')
    })
  }

  it('confirms a sent request', () => {
    submission = { ...idleSubmission, isSuccess: true }

    const html = render(BROWSEROS_ID)

    expect(html).toContain('DM us on Discord with your screenshot')
  })

  it('surfaces the failure message and keeps the form usable', () => {
    submission = {
      ...idleSubmission,
      isError: true,
      error: new Error('Something went wrong, please try again.'),
    }

    const html = render(BROWSEROS_ID)

    expect(html).toContain('Something went wrong, please try again.')
    expect(html).toContain('id="discord-handle"')
    expect(html).toContain('Submit')
  })
})
