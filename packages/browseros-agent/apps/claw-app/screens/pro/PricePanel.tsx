/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan, PlanPrice } from '@browseros/claw-api'
import { Check, ExternalLink, Info, KeyRound, LoaderCircle } from 'lucide-react'
import type { ReactNode } from 'react'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import { trialEndFrom } from '@/modules/plan/trial.helpers'
import {
  apiErrorMessage,
  BACKING_LINE,
  type BillingPeriod,
  billingPeriods,
  formatDate,
  formatPrice,
  isBillingPeriod,
  keyHasEnded,
  monthlyEquivalent,
  monthsFree,
  monthsFreeLabel,
  PLAN_UNCHECKED,
  type ProPrices,
  perPeriod,
} from './pro.helpers'
import type { ProCheckout } from './pro.hooks'

interface PricePanelProps {
  plan: Plan
  pricing: { isPending: boolean; isError: boolean; error: unknown }
  prices: ProPrices | null
  /** The free trial the copy promises for Monthly: the checkout's grant once one started, or 0. */
  trialDays: number
  /** The free trial the price list offers for Monthly, or 0. */
  offeredTrialDays: number
  period: BillingPeriod
  onPeriodChange: (period: BillingPeriod) => void
  checkout: ProCheckout
  onBuy: () => void
  onUseKey: () => void
}

/** What Free sees: the price, Get ultrafast, and the way in for someone who already paid. */
export function PricePanel({
  plan,
  pricing,
  prices,
  trialDays,
  offeredTrialDays,
  period,
  onPeriodChange,
  checkout,
  onBuy,
  onUseKey,
}: PricePanelProps) {
  const ended = keyHasEnded(plan)
  // Someone who paid on another computer needs only their key, never the price.
  const useKey = ended ? null : <UseKeyLink onUseKey={onUseKey} />

  if (pricing.isPending) {
    return (
      <>
        <div className="flex w-full flex-col gap-4">
          <Skeleton className="h-9 w-56" />
          <Skeleton className="h-12 w-40" />
          <Skeleton className="h-[46px] w-44" />
        </div>
        {useKey}
      </>
    )
  }
  const price = prices ? (prices[period] ?? prices.month ?? prices.year) : null
  if (pricing.isError || !prices || !price) {
    return (
      <>
        <p role="alert" className="text-[14.5px] text-ink-2">
          {apiErrorMessage(pricing.error) ?? PLAN_UNCHECKED}
        </p>
        {useKey}
      </>
    )
  }

  const amount = formatPrice(price)
  const trial = trialDays > 0 && price.interval === 'month'

  if (checkout.phase === 'waiting' || checkout.phase === 'confirming') {
    return (
      <WaitingPanel
        amount={amount}
        trial={(checkout.trialDays ?? 0) > 0}
        // Offered, but the gateway said no at checkout: not eligible, or trials are paused.
        trialRefused={
          offeredTrialDays > 0 &&
          price.interval === 'month' &&
          checkout.trialDays === 0
        }
        paid={checkout.phase === 'confirming'}
        onReopen={checkout.reopen}
        onCancel={checkout.cancel}
      />
    )
  }

  return (
    <>
      {checkout.phase === 'abandoned' ? (
        <Note icon={<Info />}>No payment was made.</Note>
      ) : ended ? (
        <Note icon={<KeyRound />} warn>
          Renew and your key works again.
        </Note>
      ) : null}

      {billingPeriods(prices).length > 1 ? (
        <PeriodSwitch
          period={period}
          saving={monthsFree(prices)}
          trialDays={trialDays}
          onPeriodChange={onPeriodChange}
        />
      ) : null}

      <PriceLine price={price} amount={amount} />

      <BuyButton
        isStarting={checkout.isStarting}
        ended={ended}
        trial={trial}
        onBuy={onBuy}
      />
      {/* Monthly and Yearly read the same: the terms, then the thanks in
          bold, in one paragraph, with the way in by key on its own line. */}
      <p className="max-w-[46ch] text-[14px] text-ink-2">
        {trial ? (
          <>
            Free until{' '}
            <strong className="text-cyanotype-ink">
              {formatDate(trialEndFrom(Date.now(), trialDays))}
            </strong>
            , then {amount} a month. Cancel before then and you won’t be
            charged.
          </>
        ) : (
          <>
            {price.interval === 'year'
              ? `Works out to ${monthlyEquivalent(price)} a month. `
              : null}
            Cancel any time.
            {trialDays > 0 ? ' The free trial comes with Monthly.' : null}
          </>
        )}{' '}
        <strong className="text-cyanotype-ink">{BACKING_LINE}</strong>
      </p>
      {useKey ? <p className="text-[13.5px]">{useKey}</p> : null}
    </>
  )
}

function UseKeyLink({ onUseKey }: { onUseKey: () => void }) {
  return (
    <button
      type="button"
      onClick={onUseKey}
      className="rounded-sm font-semibold text-[13.5px] text-accent-ink underline-offset-4 outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
    >
      Use a licence key
    </button>
  )
}

/** Get ultrafast, Start free trial while one is offered, or Renew ultrafast once a key's plan has ended. */
export function BuyButton({
  isStarting,
  ended,
  trial = false,
  onBuy,
}: {
  isStarting: boolean
  ended: boolean
  trial?: boolean
  onBuy: () => void
}) {
  const label = ended
    ? 'Renew ultrafast'
    : trial
      ? 'Start free trial'
      : 'Get ultrafast'
  return (
    <StatefulButton
      state={isStarting ? 'loading' : 'idle'}
      loadingText={label}
      disabled={isStarting}
      onClick={onBuy}
      className="h-[46px] px-[26px] font-semibold text-[15.5px] max-sm:w-full"
    >
      {label}
    </StatefulButton>
  )
}

// ---------------------------------------------------------------------------

function PeriodSwitch({
  period,
  saving,
  trialDays,
  onPeriodChange,
}: {
  period: BillingPeriod
  saving: number | null
  trialDays: number
  onPeriodChange: (period: BillingPeriod) => void
}) {
  return (
    <Tabs
      variant="segment"
      value={period}
      onValueChange={(value) => {
        if (isBillingPeriod(value)) onPeriodChange(value)
      }}
      className="self-start"
    >
      <TabsList aria-label="Billing period" className="border border-border-2">
        <TabsTrigger value="month" className="font-semibold text-[13.5px]">
          Monthly
          {trialDays > 0 ? (
            <span className="ml-1.5 rounded-full bg-green-tint px-1.5 font-bold text-[11px] text-green-ink leading-[18px]">
              {trialDays} days free
            </span>
          ) : null}
        </TabsTrigger>
        <TabsTrigger value="year" className="font-semibold text-[13.5px]">
          Yearly
          {saving ? (
            <span className="ml-1.5 rounded-full bg-green-tint px-1.5 font-bold text-[11px] text-green-ink leading-[18px]">
              {monthsFreeLabel(saving)}
            </span>
          ) : null}
        </TabsTrigger>
      </TabsList>
    </Tabs>
  )
}

function PriceLine({ price, amount }: { price: PlanPrice; amount: string }) {
  const period: BillingPeriod = price.interval === 'year' ? 'year' : 'month'
  return (
    <div className="mt-1 flex items-baseline gap-1.5 text-cyanotype-ink">
      <strong className="font-extrabold text-[52px] tabular-nums leading-none tracking-[-0.035em]">
        {amount}
      </strong>
      <span className="font-medium text-base text-ink-2">
        {perPeriod(period)}
      </span>
    </div>
  )
}

/** A one-line note in the hero, with no box around it. */
function Note({
  icon,
  warn = false,
  children,
}: {
  icon: ReactNode
  warn?: boolean
  children: ReactNode
}) {
  return (
    <p
      role="status"
      className="flex items-center gap-2 font-semibold text-[14.5px] text-ink-2"
    >
      <span
        aria-hidden
        className={cn(
          'shrink-0 [&>svg]:size-4',
          warn ? 'text-amber-ink' : 'text-ink-3',
        )}
      >
        {icon}
      </span>
      {children}
    </p>
  )
}

type StepState = 'done' | 'now' | 'next'

function WaitingPanel({
  amount,
  trial,
  trialRefused,
  paid,
  onReopen,
  onCancel,
}: {
  amount: string
  /** The checkout starts a free trial: a card now, the first charge later. */
  trial: boolean
  trialRefused: boolean
  paid: boolean
  onReopen: () => void
  onCancel: () => void
}) {
  const steps: { label: string; state: StepState }[] = [
    { label: 'Payment page opened', state: 'done' },
    {
      label: trial ? 'Add your card with Stripe' : `Pay ${amount} with Stripe`,
      state: paid ? 'done' : 'now',
    },
    {
      label: trial ? 'Your free trial starts here' : 'ultrafast turns on here',
      state: paid ? 'now' : 'next',
    },
  ]
  return (
    <div role="status" className="flex w-full flex-col items-start gap-4">
      <h2 className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]">
        {paid
          ? trial
            ? 'Card added'
            : 'Payment received'
          : 'Finish in the payment tab'}
      </h2>
      {trialRefused ? (
        <Note icon={<Info />}>Trial temporarily unavailable</Note>
      ) : null}
      <ol className="flex w-full max-w-[380px] flex-col gap-3.5">
        {steps.map((step) => (
          <li
            key={step.label}
            className={cn(
              'grid grid-cols-[28px_minmax(0,1fr)] items-center gap-3 text-[15px]',
              step.state === 'next' ? 'text-ink-3' : 'text-cyanotype-ink',
              step.state === 'now' && 'font-semibold',
            )}
          >
            <span
              aria-hidden
              className={cn(
                'grid size-7 place-items-center rounded-full border-[1.5px]',
                step.state === 'done'
                  ? 'border-green bg-green text-white'
                  : step.state === 'now'
                    ? 'border-primary text-primary'
                    : 'border-border-strong',
              )}
            >
              {step.state === 'done' ? (
                <Check className="size-4" />
              ) : step.state === 'now' ? (
                <LoaderCircle className="size-4 animate-spin" />
              ) : null}
            </span>
            {step.label}
          </li>
        ))}
      </ol>
      {paid ? null : (
        <>
          <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
            <Button
              variant="secondary"
              onClick={onReopen}
              className="font-semibold"
            >
              <ExternalLink aria-hidden className="size-4" />
              Open payment page
            </Button>
            <Button
              variant="ghost"
              onClick={onCancel}
              className="font-semibold"
            >
              Cancel
            </Button>
          </div>
          <p className="text-[13.5px] text-ink-3">
            {trial
              ? 'You won’t be charged today.'
              : 'Nothing is charged until you subscribe.'}
          </p>
        </>
      )}
    </div>
  )
}
