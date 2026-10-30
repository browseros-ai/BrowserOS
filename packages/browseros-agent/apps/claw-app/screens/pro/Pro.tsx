/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan } from '@browseros/claw-api'
import { ChevronDown } from 'lucide-react'
import { useState } from 'react'
import { Navigate } from 'react-router'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { Card } from '@/components/motion/card'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleIndicator,
  CollapsibleTrigger,
} from '@/components/motion/collapsible'
import { CancelProDialog } from '@/components/pro/CancelProDialog'
import { KeySection } from '@/components/pro/KeySection'
import { UseKeyDialog } from '@/components/pro/UseKeyDialog'
import { WelcomeProDialog } from '@/components/pro/WelcomeProDialog'
import { Skeleton } from '@/components/ui/skeleton'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { PlanPanel } from './PlanPanel'
import { PricePanel } from './PricePanel'
import { useProData } from './pro.data'
import {
  apiErrorMessage,
  type BillingPeriod,
  formatPrice,
  isPro,
  keyHasEnded,
  PLAN_UNCHECKED,
  PRO_FEATURES,
  type ProPrices,
  pickPeriod,
} from './pro.helpers'

const TITLE = (
  <>
    neo <em className="text-accent-ink">Pro</em>
  </>
)

/** Everything about Pro in one place: what it adds, buying it, the key and the plan. */
export function Pro() {
  const { plan, pricing, prices, planKey, checkout, portal } = useProData()
  const [wantedPeriod, setWantedPeriod] = useState<BillingPeriod>('month')
  const [keyDialogOpen, setKeyDialogOpen] = useState(false)
  const [keyDialogOpens, setKeyDialogOpens] = useState(0)
  const [cancelOpen, setCancelOpen] = useState(false)

  if (plan.isPending) {
    return (
      <Page>
        <PageHeader title={TITLE} />
        <Skeleton className="h-80 w-full rounded-2xl" />
      </Page>
    )
  }
  if (plan.isError) {
    return (
      <Page>
        <PageHeader title={TITLE} />
        <p role="alert" className="text-[14.5px] text-ink-2">
          {apiErrorMessage(plan.error) ?? PLAN_UNCHECKED}
        </p>
      </Page>
    )
  }
  if (!plan.data.available) return <Navigate to="/" replace />

  const current = plan.data
  const pro = isPro(current)
  const period = prices ? pickPeriod(prices, wantedPeriod) : wantedPeriod

  const buy = () => {
    const price = prices?.[period]
    if (!prices || !price) return
    checkout.start({
      plan: prices.plan,
      interval: price.interval,
      sameKey: keyHasEnded(current),
    })
  }

  return (
    <Page>
      <PageHeader title={TITLE} lede={lede(current, prices)} />
      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1.35fr)_minmax(300px,1fr)]">
        <FeatureList />
        <Card
          aria-live="polite"
          className="gap-[18px] border-border-2 bg-card-tint p-6 shadow-card lg:sticky lg:top-6"
        >
          {pro ? (
            <PlanPanel
              plan={current}
              planKey={planKey.data}
              portal={portal}
              onCancel={() => setCancelOpen(true)}
            />
          ) : (
            <PricePanel
              plan={current}
              pricing={pricing}
              prices={prices}
              period={period}
              onPeriodChange={setWantedPeriod}
              checkout={checkout}
              onBuy={buy}
              onUseKey={() => {
                setKeyDialogOpens((count) => count + 1)
                setKeyDialogOpen(true)
                track(AnalyticsEvent.ProKeyDialogOpened)
              }}
            />
          )}
        </Card>
      </div>
      {pro && current.onKey ? <KeySection planKey={planKey} /> : null}
      <ProQuestions prices={prices} />

      <UseKeyDialog
        key={keyDialogOpens}
        open={keyDialogOpen}
        onClose={() => setKeyDialogOpen(false)}
        onGetProAgain={buy}
      />
      {pro ? (
        <CancelProDialog
          plan={current}
          open={cancelOpen}
          onOpenChange={setCancelOpen}
        />
      ) : null}
      <WelcomeProDialog
        open={checkout.welcome !== null}
        welcome={checkout.welcome}
        onClose={checkout.closeWelcome}
      />
    </Page>
  )
}

function lede(plan: Plan, prices: ProPrices | null): string | null {
  if (isPro(plan)) {
    return 'Thanks for supporting BrowserOS. Here’s what Pro adds to neo.'
  }
  if (!prices?.month) return null
  const yearly = prices.year ? ', or less if you pay yearly' : ''
  return `More from your AI agents, for ${formatPrice(prices.month)} a month${yearly}. Cancel whenever you like.`
}

// ---------------------------------------------------------------------------

function FeatureList() {
  return (
    <Card
      aria-label="What Pro adds"
      className="gap-0 border-border-2 bg-card py-2 shadow-card"
    >
      <ul className="flex flex-col divide-y divide-border-2">
        {PRO_FEATURES.map((feature) => (
          <li
            key={feature.title}
            className="grid grid-cols-[44px_minmax(0,1fr)] gap-4 px-6 py-[18px]"
          >
            <span
              aria-hidden
              className="grid size-11 place-items-center rounded-xl bg-accent-tint text-primary"
            >
              <feature.icon className="size-5" />
            </span>
            <div>
              <div className="font-bold text-base text-cyanotype-ink">
                {feature.title}
              </div>
              <p className="mt-0.5 text-[14.5px] text-ink-2">
                {feature.description}
              </p>
            </div>
          </li>
        ))}
      </ul>
    </Card>
  )
}

function ProQuestions({ prices }: { prices: ProPrices | null }) {
  const both = Boolean(prices?.month && prices.year)
  const questions = [
    {
      q: 'When am I charged?',
      a: `Today, and then on the same date every month${both ? ', or every year if you pay yearly' : ''}. Stripe emails you a receipt each time.`,
    },
    ...(both
      ? [
          {
            q: 'Can I switch between monthly and yearly?',
            a: 'Yes. Choose Billing and receipts on this page and change your plan there. Going yearly starts right away, and you only pay the difference. Going back to monthly starts when your paid year ends.',
          },
        ]
      : []),
    {
      q: 'How do I cancel?',
      a: 'Come back to this page and choose Cancel Pro. You keep Pro until the end of the month you already paid for, and you are not charged again.',
    },
    {
      q: 'I reinstalled neo or got a new computer.',
      a: 'Choose Use a licence key and paste your key. Pro turns on right away.',
    },
    {
      q: 'Can I use Pro on more than one computer?',
      a: 'Yes, on up to 5 computers with one key. You can see them all, and remove any, on this page.',
    },
    {
      q: 'Where’s my licence key?',
      a: 'In the email we sent when you paid. Search for “neo Pro licence key”. Can’t find it? Reset your key on this page and we’ll email a new one.',
    },
    {
      q: 'What if someone else gets my key?',
      a: 'Choose Reset key on this page. You get a new key, and Pro turns off on every other computer until you paste the new key there.',
    },
    {
      q: 'Who sees my card details?',
      a: 'Only Stripe, the payment company that runs the checkout page. neo and BrowserOS never see or store your card.',
    },
  ]
  return (
    <section
      aria-label="Questions people ask"
      className="flex flex-col gap-3.5"
    >
      <h2 className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]">
        Questions people ask
      </h2>
      <Card
        size="compact"
        className="gap-0 border-border-2 bg-card py-1 shadow-card"
      >
        <div className="flex flex-col divide-y divide-border-2">
          {questions.map(({ q, a }, index) => (
            <Collapsible key={q} defaultOpen={index === 0}>
              <CollapsibleTrigger className="flex w-full items-center justify-between gap-3 px-[22px] py-4 text-left font-semibold text-[15.5px] text-cyanotype-ink outline-none focus-visible:ring-2 focus-visible:ring-ring">
                {q}
                <CollapsibleIndicator>
                  <ChevronDown aria-hidden className="size-4 text-ink-3" />
                </CollapsibleIndicator>
              </CollapsibleTrigger>
              <CollapsibleContent contentClassName="px-[22px] pb-4">
                <p className="max-w-[70ch] text-[14.5px] text-ink-2">{a}</p>
              </CollapsibleContent>
            </Collapsible>
          ))}
        </div>
      </Card>
    </section>
  )
}
