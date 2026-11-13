/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

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
import {
  StepRacePicture,
  UltrafastPicture,
} from '@/components/pro/UltrafastPictures'
import { UltrafastTour } from '@/components/pro/UltrafastTour'
import { UseKeyDialog } from '@/components/pro/UseKeyDialog'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { PlanPanel } from './PlanPanel'
import { BuyButton, PricePanel } from './PricePanel'
import { useProData } from './pro.data'
import {
  apiErrorMessage,
  type BillingPeriod,
  formatDate,
  isPro,
  keyHasEnded,
  PLAN_UNCHECKED,
  type ProPrices,
  pickPeriod,
  ULTRAFAST_STEPS,
} from './pro.helpers'
import { SavingsHero } from './SavingsHero'

const TITLE = (
  <>
    <em className="text-accent-ink">ultrafast</em>
  </>
)

/**
 * Everything about Pro in one place. Every state follows one rule: a plain
 * hero with no panel, only the feature tiles are boxes, and the questions are
 * a plain list.
 */
export function Pro() {
  const { plan, pricing, prices, planKey, savings, checkout, portal } =
    useProData()
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
  const ended = keyHasEnded(current)
  const period = prices ? pickPeriod(prices, wantedPeriod) : wantedPeriod
  const idle = checkout.phase !== 'waiting' && checkout.phase !== 'confirming'

  const buy = () => {
    const price = prices?.[period]
    if (!prices || !price) return
    checkout.start({ plan: prices.plan, interval: price.interval })
  }

  return (
    <Page className="gap-12 md:gap-16">
      {pro ? (
        <>
          <PageHeader title={TITLE} />
          <SavingsHero
            savings={savings}
            onShowMe={() => {
              setTourOpens((count) => count + 1)
              setTourAt(0)
            }}
          />
          <PlanPanel
            plan={current}
            portal={portal}
            onCancel={() => setCancelOpen(true)}
          />
          {current.onKey ? <KeySection planKey={planKey} /> : null}
          <FeatureTiles
            all
            onShowMe={(step) => {
              setTourOpens((count) => count + 1)
              setTourAt(step)
            }}
          />
        </>
      ) : (
        <>
          <section
            aria-label="Get ultrafast"
            className="grid items-center gap-4 lg:grid-cols-2 lg:gap-12"
          >
            <div className="flex min-w-0 flex-col items-start gap-3.5">
              <PageHeader title={TITLE} />
              <p className="max-w-[16ch] font-extrabold text-[28px] text-cyanotype-ink leading-[1.12] tracking-[-0.03em] md:text-[34px]">
                More from your AI agents.
              </p>
              <div
                aria-live="polite"
                className="mt-3.5 flex w-full flex-col items-start gap-4"
              >
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
              </div>
            </div>
            <div className="relative isolate min-w-0 py-4 before:pointer-events-none before:absolute before:inset-x-0 before:-inset-y-6 before:-z-10 before:bg-[radial-gradient(closest-side,var(--color-accent-tint),transparent)] sm:px-2 sm:py-7 lg:before:-inset-x-6 lg:before:-inset-y-10">
              <StepRacePicture large />
            </div>
          </section>
          <FeatureTiles />
          {idle && prices ? (
            <section
              aria-label="Get ultrafast again"
              className="flex flex-wrap items-center justify-between gap-x-6 gap-y-4"
            >
              <h2 className="font-extrabold text-[22px] text-cyanotype-ink tracking-[-0.025em] md:text-[26px]">
                {ended
                  ? 'Turn ultrafast back on'
                  : 'Make your agents ultrafast'}
              </h2>
              <BuyButton
                isStarting={checkout.isStarting}
                ended={ended}
                onBuy={buy}
              />
            </section>
          ) : null}
        </>
      )}
      <ProQuestions
        prices={prices}
        on={pro}
        nextPayment={current.status === 'active' ? current.periodEnd : null}
      />

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

// ---------------------------------------------------------------------------

/**
 * What ultrafast adds, one box each. Before buying, the hero already shows
 * browsing, so only the other two are here.
 */
function FeatureTiles({
  all = false,
  onShowMe,
}: {
  all?: boolean
  /** Set once ultrafast is on: opens the welcome at that picture. */
  onShowMe?: (step: number) => void
}) {
  const shown = ULTRAFAST_STEPS.map((step, index) => ({ step, index })).filter(
    ({ index }) => all || index > 0,
  )
  return (
    <section
      aria-labelledby="ultrafast-features"
      className="flex flex-col gap-3.5"
    >
      <h2
        id="ultrafast-features"
        className={cn(
          'font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]',
          !all && 'sr-only',
        )}
      >
        What’s in ultrafast
      </h2>
      <div className="grid gap-4 md:grid-cols-2">
        {shown.map(({ step, index }) => {
          const wide = all && index === 0
          return (
            <Card
              key={step.name}
              className={cn(
                'gap-0 overflow-hidden border-border-2 bg-card p-0 shadow-card',
                wide &&
                  'md:col-span-2 lg:grid lg:grid-cols-[minmax(0,0.9fr)_minmax(0,1.1fr)]',
              )}
            >
              <div
                className={cn(
                  'flex flex-col items-start gap-2 px-6 pt-[22px] pb-5',
                  wide && 'lg:justify-center',
                )}
              >
                <h3 className="flex items-center gap-2.5 font-bold text-[19px] text-cyanotype-ink leading-[1.2] tracking-[-0.015em]">
                  <step.icon
                    aria-hidden
                    className="size-5 shrink-0 text-primary"
                  />
                  {step.title}
                </h3>
                {onShowMe ? (
                  <button
                    type="button"
                    aria-label={`Show me ${step.name}`}
                    onClick={() => onShowMe(index)}
                    className="rounded-sm font-semibold text-[14px] text-accent-ink underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
                  >
                    Show me
                  </button>
                ) : null}
              </div>
              <div
                className={cn(
                  'flex-1 border-border-2 border-t bg-accent-tint px-6 py-5',
                  wide && 'lg:flex lg:items-center lg:border-t-0 lg:border-l',
                )}
              >
                <UltrafastPicture step={index} />
              </div>
            </Card>
          )
        })}
      </div>
    </section>
  )
}

function ProQuestions({
  prices,
  on,
  nextPayment,
}: {
  prices: ProPrices | null
  on: boolean
  nextPayment: number | null
}) {
  const yearly = prices?.year ? ', or each year if you pay yearly' : ''
  const charged =
    nextPayment !== null
      ? `Next on ${formatDate(nextPayment)}. Stripe emails you a receipt each time.`
      : on
        ? null
        : `Today, then on the same date each month${yearly}.`
  const questions = [
    ...(charged ? [{ q: 'When am I charged?', a: charged }] : []),
    {
      q: 'What if I cancel?',
      a: 'ultrafast stays on until your paid time ends. Your skills keep working, with your agent at normal speed.',
    },
    {
      q: 'Does neo change sites without asking?',
      a: 'No. For skills that change a site, like adding to a cart, you choose how they run. Your agent asks you first.',
    },
    ...(on
      ? [
          {
            q: 'New computer, or reinstalled neo?',
            a: 'Choose Use a licence key and paste your key.',
          },
        ]
      : []),
    {
      q: 'More than one computer?',
      a: on
        ? 'Yes, up to 5 with one key. They’re listed above.'
        : 'Yes, up to 5 with one key.',
    },
    ...(on
      ? []
      : [
          {
            q: 'Who sees my card?',
            a: 'Only Stripe. neo never sees or stores it.',
          },
        ]),
  ]
  return (
    <section
      aria-labelledby="ultrafast-questions"
      className="flex flex-col gap-3.5"
    >
      <h2
        id="ultrafast-questions"
        className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]"
      >
        Questions
      </h2>
      <div className="flex flex-col divide-y divide-border-2 border-border-2 border-y">
        {questions.map(({ q, a }) => (
          <Collapsible key={q}>
            <CollapsibleTrigger className="flex w-full items-center justify-between gap-3 py-[18px] text-left font-semibold text-[15px] text-cyanotype-ink outline-none focus-visible:ring-2 focus-visible:ring-ring">
              {q}
              <CollapsibleIndicator>
                <ChevronDown aria-hidden className="size-4 text-ink-3" />
              </CollapsibleIndicator>
            </CollapsibleTrigger>
            <CollapsibleContent contentClassName="pb-[18px]">
              <p className="max-w-[70ch] text-[14.5px] text-ink-2">{a}</p>
            </CollapsibleContent>
          </Collapsible>
        ))}
      </div>
    </section>
  )
}
