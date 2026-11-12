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
import { UltrafastPicture } from '@/components/pro/UltrafastPictures'
import { UltrafastTour } from '@/components/pro/UltrafastTour'
import { UseKeyDialog } from '@/components/pro/UseKeyDialog'
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
  PLAN_UNCHECKED,
  type ProPrices,
  pickPeriod,
  ULTRAFAST_STEPS,
} from './pro.helpers'

const TITLE = (
  <>
    <em className="text-accent-ink">ultrafast</em>
  </>
)

/** Everything about Pro in one place: what it adds, buying it, the key and the plan. */
export function Pro() {
  const { plan, pricing, prices, planKey, checkout, portal } = useProData()
  const [wantedPeriod, setWantedPeriod] = useState<BillingPeriod>('month')
  const [keyDialogOpen, setKeyDialogOpen] = useState(false)
  const [keyDialogOpens, setKeyDialogOpens] = useState(0)
  const [cancelOpen, setCancelOpen] = useState(false)
  const [tourAt, setTourAt] = useState<number | null>(null)
  const [tourOpens, setTourOpens] = useState(0)

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
    checkout.start({ plan: prices.plan, interval: price.interval })
  }

  return (
    <Page>
      <PageHeader title={TITLE} lede={lede(current, prices)} />
      <div className="grid items-start gap-6 lg:grid-cols-[minmax(0,1.35fr)_minmax(300px,1fr)]">
        <PictureTiles
          onShowMe={
            pro
              ? (step) => {
                  setTourOpens((count) => count + 1)
                  setTourAt(step)
                }
              : undefined
          }
        />
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
      <UltrafastTour
        key={`tour-${tourOpens}`}
        open={checkout.welcoming || tourAt !== null}
        startAt={tourAt ?? 0}
        welcome={checkout.welcoming}
        onClose={() => {
          if (checkout.welcoming) checkout.closeWelcome()
          setTourAt(null)
        }}
      />
    </Page>
  )
}

function lede(plan: Plan, prices: ProPrices | null): string | null {
  if (isPro(plan)) {
    return 'Thanks for supporting BrowserOS. Here’s what ultrafast adds to neo.'
  }
  if (!prices?.month) return null
  const yearly = prices.year ? ', or less if you pay yearly' : ''
  return `More from your AI agents, for ${formatPrice(prices.month)} a month${yearly}. Cancel whenever you like.`
}

// ---------------------------------------------------------------------------

/** What ultrafast adds: the welcome's three pictures, each with its headline. */
function PictureTiles({
  onShowMe,
}: {
  /** Set once ultrafast is on: opens the welcome at that picture. */
  onShowMe?: (step: number) => void
}) {
  return (
    <section aria-label="What ultrafast adds" className="flex flex-col gap-4">
      {ULTRAFAST_STEPS.map((step, index) => (
        <Card
          key={step.name}
          className="gap-0 overflow-hidden border-border-2 bg-card p-0 shadow-card"
        >
          <div className="flex flex-col items-start gap-1.5 px-[22px] pt-5 pb-4">
            <span className="inline-flex items-center gap-2 font-semibold text-[13.5px] text-accent-ink">
              <step.icon aria-hidden className="size-4" />
              {step.name}
            </span>
            <h2 className="font-bold text-[21px] text-cyanotype-ink leading-[1.2] tracking-[-0.015em]">
              {step.title}
            </h2>
            <p className="text-[15px] text-ink-2">{step.line}</p>
            {onShowMe ? (
              <button
                type="button"
                aria-label={`Show me ${step.name}`}
                onClick={() => onShowMe(index)}
                className="mt-1 rounded-sm font-semibold text-[14px] text-accent-ink underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
              >
                Show me
              </button>
            ) : null}
          </div>
          <div className="border-border-2 border-t bg-accent-tint px-[22px] py-5">
            <UltrafastPicture step={index} />
          </div>
        </Card>
      ))}
    </section>
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
      a: 'Come back to this page and choose Cancel ultrafast. You keep ultrafast until the end of the month you already paid for, and you are not charged again. After that, your skills still work, with your agent at normal speed.',
    },
    {
      q: 'Does neo change things on sites without asking?',
      a: 'No. When a skill adds to a cart or changes anything else on a site, you choose how it runs: with your agent checking each page, or without it. Your agent asks you the first time. You can change it on the skill’s page.',
    },
    {
      q: 'I reinstalled neo or got a new computer.',
      a: 'Choose Use a licence key and paste your key. ultrafast turns on right away.',
    },
    {
      q: 'Can I use ultrafast on more than one computer?',
      a: 'Yes, on up to 5 computers with one key. You can see them all, and remove any, on this page.',
    },
    {
      q: 'Where’s my licence key?',
      a: 'In the email we sent when you paid. Search for “ultrafast licence key”. Can’t find it? Reset your key on this page and we’ll email a new one.',
    },
    {
      q: 'What if someone else gets my key?',
      a: 'Choose Reset key on this page. You get a new key, and ultrafast turns off on every other computer until you paste the new key there.',
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
