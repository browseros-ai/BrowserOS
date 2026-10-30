/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Plan } from '@browseros/claw-api'
import {
  Check,
  ExternalLink,
  Info,
  KeyRound,
  LoaderCircle,
  Lock,
} from 'lucide-react'
import type { ReactNode } from 'react'
import {
  Alert,
  AlertContent,
  AlertDescription,
} from '@/components/motion/alert'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import {
  apiErrorMessage,
  type BillingPeriod,
  billingPeriods,
  eachPeriod,
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
  period: BillingPeriod
  onPeriodChange: (period: BillingPeriod) => void
  checkout: ProCheckout
  onBuy: () => void
  onUseKey: () => void
}

/** What Free sees: the price, Get Pro, and the way in for someone who already paid. */
export function PricePanel({
  plan,
  pricing,
  prices,
  period,
  onPeriodChange,
  checkout,
  onBuy,
  onUseKey,
}: PricePanelProps) {
  if (pricing.isPending) {
    return (
      <div className="flex flex-col gap-4">
        <Skeleton className="h-12 w-40" />
        <Skeleton className="h-24 w-full" />
        <Skeleton className="h-10 w-full" />
      </div>
    )
  }
  if (pricing.isError || !prices) {
    return (
      <p role="alert" className="text-[14.5px] text-ink-2">
        {apiErrorMessage(pricing.error) ?? PLAN_UNCHECKED}
      </p>
    )
  }

  const price = prices[period] ?? prices.month ?? prices.year
  if (!price) return null
  const amount = formatPrice(price)

  if (checkout.phase === 'waiting' || checkout.phase === 'confirming') {
    return (
      <WaitingPanel
        amount={amount}
        paid={checkout.phase === 'confirming'}
        onReopen={checkout.reopen}
        onCancel={checkout.cancel}
      />
    )
  }

  const saving = monthsFree(prices)
  const periods = billingPeriods(prices)

  return (
    <>
      {checkout.phase === 'abandoned' ? (
        <PanelNote icon={<Info />}>
          No payment was made. You can try again whenever you like.
        </PanelNote>
      ) : keyHasEnded(plan) ? (
        <PanelNote icon={<KeyRound />} tone="warn">
          This computer’s licence key has no Pro right now. Get Pro to start it
          again.
        </PanelNote>
      ) : null}

      {periods.length > 1 ? (
        <Tabs
          variant="segment"
          value={period}
          onValueChange={(value) => {
            if (isBillingPeriod(value)) onPeriodChange(value)
          }}
          className="self-start"
        >
          <TabsList
            aria-label="Billing period"
            className="border border-border-2"
          >
            <TabsTrigger value="month" className="font-semibold text-[13.5px]">
              Monthly
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
      ) : null}

      <div className="flex flex-col gap-1">
        <div className="flex items-baseline gap-1.5 text-cyanotype-ink">
          <strong className="font-extrabold text-[44px] tabular-nums leading-none tracking-[-0.035em]">
            {amount}
          </strong>
          <span className="font-medium text-base text-ink-2">
            {perPeriod(price.interval === 'year' ? 'year' : 'month')}
          </span>
        </div>
        {price.interval === 'year' ? (
          <p className="text-[13px] text-ink-3">
            Works out to {monthlyEquivalent(price)} a month.
          </p>
        ) : null}
      </div>

      <ul className="flex flex-col gap-2.5 text-[14.5px] text-ink">
        {[
          'Everything in Pro, starting today',
          `Renews by itself ${eachPeriod(price.interval === 'year' ? 'year' : 'month')}`,
          'Cancel right here, any time',
        ].map((line) => (
          <li key={line} className="flex items-start gap-2.5">
            <Check aria-hidden className="mt-1 size-4 shrink-0 text-green" />
            {line}
          </li>
        ))}
      </ul>

      <StatefulButton
        state={checkout.isStarting ? 'loading' : 'idle'}
        loadingText="Get Pro"
        disabled={checkout.isStarting}
        onClick={onBuy}
        className="w-full font-semibold"
      >
        Get Pro
      </StatefulButton>
      <p className="flex items-start gap-2 text-[13px] text-ink-3">
        <Lock aria-hidden className="mt-0.5 size-3.5 shrink-0" />
        <span>You pay on Stripe’s secure page. neo never sees your card.</span>
      </p>
      <div className="flex items-center gap-3 text-[13px] text-ink-3 before:h-px before:flex-1 before:bg-border-2 after:h-px after:flex-1 after:bg-border-2">
        Already paid?
      </div>
      <Button
        variant="secondary"
        onClick={onUseKey}
        className="w-full font-semibold"
      >
        <KeyRound aria-hidden className="size-4" />
        Use a licence key
      </Button>
    </>
  )
}

// ---------------------------------------------------------------------------

function PanelNote({
  icon,
  tone = 'info',
  children,
}: {
  icon: ReactNode
  tone?: 'info' | 'warn'
  children: ReactNode
}) {
  return (
    <Alert
      variant={tone === 'warn' ? 'warning' : 'default'}
      role="status"
      className={cn(
        'rounded-xl px-3.5 py-3',
        tone === 'warn'
          ? 'border-transparent bg-amber-tint text-amber-ink'
          : 'border-border-2 bg-card text-ink-2',
      )}
      contentClassName="items-start gap-2.5"
    >
      <span aria-hidden className="mt-0.5 shrink-0 [&>svg]:size-4">
        {icon}
      </span>
      <AlertContent className="space-y-0">
        <AlertDescription className="text-[14px]">{children}</AlertDescription>
      </AlertContent>
    </Alert>
  )
}

type StepState = 'done' | 'now' | 'next'

function WaitingPanel({
  amount,
  paid,
  onReopen,
  onCancel,
}: {
  amount: string
  paid: boolean
  onReopen: () => void
  onCancel: () => void
}) {
  const steps: { label: string; state: StepState }[] = [
    { label: 'Payment page opened', state: 'done' },
    { label: `Pay ${amount} with Stripe`, state: paid ? 'done' : 'now' },
    { label: 'Pro turns on here', state: paid ? 'now' : 'next' },
  ]
  return (
    <div role="status" className="flex flex-col gap-5">
      <div>
        <div className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]">
          {paid ? 'Payment received' : 'Finish in the payment tab'}
        </div>
        <p className="mt-1 text-[14.5px] text-ink-2">
          {paid
            ? 'One moment while neo turns on Pro.'
            : 'We opened Stripe’s secure payment page in a new tab.'}
        </p>
      </div>
      <ol className="flex flex-col gap-3.5">
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
                    ? 'border-primary bg-card text-primary'
                    : 'border-border-strong bg-card',
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
          <Button
            variant="secondary"
            onClick={onReopen}
            className="w-full font-semibold"
          >
            <ExternalLink aria-hidden className="size-4" />
            Open the payment page again
          </Button>
          <p className="text-[13.5px] text-ink-3">
            Nothing is charged until you press Subscribe on that page.{' '}
            <button
              type="button"
              onClick={onCancel}
              className="font-semibold text-accent-ink hover:underline"
            >
              Cancel
            </button>
          </p>
        </>
      )}
    </div>
  )
}
