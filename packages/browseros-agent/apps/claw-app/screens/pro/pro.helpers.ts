/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  Plan,
  PlanCheckoutStatus,
  PlanComputer,
  PlanPrice,
  PlanPricing,
} from '@browseros/claw-api'
import { ApiResponseError } from '@browseros/claw-api-client'
import {
  CalendarClock,
  History,
  LifeBuoy,
  type LucideIcon,
  Zap,
} from 'lucide-react'

export interface ProFeature {
  icon: LucideIcon
  title: string
  description: string
}

/** Placeholders until the real Pro features are decided. */
export const PRO_FEATURES: readonly ProFeature[] = [
  {
    icon: Zap,
    title: 'Ultrafast mode included',
    description: 'Browser tasks finish faster and use far fewer AI tokens.',
  },
  {
    icon: History,
    title: 'A year of history',
    description: 'Keep every task’s recording and steps for 12 months.',
  },
  {
    icon: CalendarClock,
    title: 'Tasks on a schedule',
    description:
      'Have your AI agents run a saved task every morning, on their own.',
  },
  {
    icon: LifeBuoy,
    title: 'Help from the team first',
    description:
      'Your questions go to the front of the BrowserOS support queue.',
  },
]

/** Shown when the plan or its prices can't be read right now. */
export const PLAN_UNCHECKED =
  'Couldn’t check your plan. Connect to the internet and neo checks again.'

export type BillingPeriod = 'month' | 'year'

/** The plan on sale, with its one-month and one-year prices when it has them. */
export interface ProPrices {
  plan: string
  month?: PlanPrice
  year?: PlanPrice
}

export function proPrices(pricing: PlanPricing): ProPrices | null {
  const sold =
    pricing.plans.find((plan) => plan.plan === 'pro') ?? pricing.plans[0]
  if (!sold) return null
  const single = (interval: BillingPeriod) =>
    sold.prices.find(
      (price) => price.interval === interval && price.intervalCount === 1,
    )
  const prices = {
    plan: sold.plan,
    month: single('month'),
    year: single('year'),
  }
  return prices.month || prices.year ? prices : null
}

/** The periods that can be bought, monthly first. */
export function billingPeriods(prices: ProPrices): BillingPeriod[] {
  const periods: BillingPeriod[] = []
  if (prices.month) periods.push('month')
  if (prices.year) periods.push('year')
  return periods
}

/** The period to show: the one picked when it is on sale, else the first that is. */
export function pickPeriod(
  prices: ProPrices,
  wanted: BillingPeriod,
): BillingPeriod {
  return prices[wanted] ? wanted : (billingPeriods(prices)[0] ?? 'month')
}

export function isBillingPeriod(value: string): value is BillingPeriod {
  return value === 'month' || value === 'year'
}

const priceFormats = new Map<string, Intl.NumberFormat>()

/** One formatter per currency and precision, since prices are formatted on every render. */
function priceFormat(currency: string, digits?: number): Intl.NumberFormat {
  const id = `${currency}:${digits ?? 'default'}`
  let format = priceFormats.get(id)
  if (!format) {
    format = new Intl.NumberFormat(undefined, {
      style: 'currency',
      currency,
      minimumFractionDigits: digits,
      maximumFractionDigits: digits,
    })
    priceFormats.set(id, format)
  }
  return format
}

/** "$20" for a whole amount, "$16.67" otherwise, from the currency's smallest unit. */
export function formatPrice(price: Pick<PlanPrice, 'amount' | 'currency'>) {
  const currency = price.currency.toUpperCase()
  const digits =
    priceFormat(currency).resolvedOptions().maximumFractionDigits ?? 2
  const value = price.amount / 10 ** digits
  return priceFormat(currency, Number.isInteger(value) ? 0 : digits).format(
    value,
  )
}

/** What a year costs per month, for "Works out to $16.67 a month." */
export function monthlyEquivalent(year: PlanPrice): string {
  return formatPrice({
    amount: Math.round(year.amount / 12),
    currency: year.currency,
  })
}

/** How many months a year saves against paying monthly, when it saves any. */
export function monthsFree(prices: ProPrices): number | null {
  if (!prices.month || !prices.year || prices.month.amount === 0) return null
  const months = Math.round(12 - prices.year.amount / prices.month.amount)
  return months > 0 ? months : null
}

export function monthsFreeLabel(months: number): string {
  return months === 1 ? '1 month free' : `${months} months free`
}

export function perPeriod(period: BillingPeriod): string {
  return period === 'year' ? 'a year' : 'a month'
}

export function eachPeriod(period: BillingPeriod): string {
  return period === 'year' ? 'each year' : 'each month'
}

export function isPro(plan: Plan): boolean {
  return plan.status !== 'none'
}

/** A key this computer is on, whose Pro has ended. */
export function keyHasEnded(plan: Plan): boolean {
  return plan.onKey && plan.status === 'none'
}

const DATE_FORMAT = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' })
// Apart from the date, because a combined format joins them with "at".
const TIME_FORMAT = new Intl.DateTimeFormat(undefined, { timeStyle: 'short' })
const RELATIVE_FORMAT = new Intl.RelativeTimeFormat(undefined, {
  numeric: 'auto',
})

/** "Nov 27, 2026", in the reader's own locale. */
export function formatDate(epochMs: number): string {
  return DATE_FORMAT.format(epochMs)
}

/** "Oct 27, 2026, 9:41 AM": exact, so two computers with one name can be told apart. */
export function formatAdded(epochMs: number): string {
  return `${formatDate(epochMs)}, ${TIME_FORMAT.format(epochMs)}`
}

const DAY_MS = 86_400_000

function startOfDay(epochMs: number): number {
  const day = new Date(epochMs)
  day.setHours(0, 0, 0, 0)
  return day.getTime()
}

/** "Last used today", "Last used 3 days ago", counted in calendar days. */
export function lastUsedLine(lastSeenAt: number, now: number): string {
  const days = Math.max(
    0,
    Math.round((startOfDay(now) - startOfDay(lastSeenAt)) / DAY_MS),
  )
  const relative = RELATIVE_FORMAT
  const when =
    days < 7
      ? relative.format(-days, 'day')
      : days < 30
        ? relative.format(-Math.round(days / 7), 'week')
        : days < 365
          ? relative.format(-Math.round(days / 30), 'month')
          : relative.format(-Math.round(days / 365), 'year')
  return `Last used ${when}`
}

const PLATFORM_NAMES: Record<string, string> = {
  macos: 'macOS',
  windows: 'Windows',
  linux: 'Linux',
}

/** The computer's name, or its system when it has none yet. */
export function computerName(computer: PlanComputer): string {
  const label = computer.label?.trim()
  if (label) return label
  return PLATFORM_NAMES[computer.platform] ?? computer.platform
}

export const COMPUTER_NAME_MAX = 40

const KEY_LETTERS = 25

/** A pasted key has NEO and then 25 letters and numbers, in any grouping. */
export function keyLooksWhole(value: string): boolean {
  const compact = value.toUpperCase().replace(/[^0-9A-Z]/g, '')
  return compact.startsWith('NEO') && compact.length === 3 + KEY_LETTERS
}

export function normalizeKey(value: string): string {
  return value.trim().toUpperCase()
}

export function keyHint(hint: string): string {
  return `NEO-${hint}`
}

export function apiErrorCode(error: unknown): string | undefined {
  return error instanceof ApiResponseError ? error.code : undefined
}

export function apiErrorMessage(error: unknown): string | undefined {
  return error instanceof ApiResponseError ? error.apiMessage : undefined
}

function isPlanComputer(value: unknown): value is PlanComputer {
  if (typeof value !== 'object' || value === null) return false
  const { installId, activatedAt, lastSeenAt } = value as Record<
    string,
    unknown
  >
  return (
    typeof installId === 'string' &&
    typeof activatedAt === 'number' &&
    typeof lastSeenAt === 'number'
  )
}

/** The computers a `key_full` refusal lists, to pick one to make room. */
export function keyFullComputers(error: unknown): PlanComputer[] {
  if (apiErrorCode(error) !== 'key_full') return []
  const computers = (error as ApiResponseError).details?.computers
  return Array.isArray(computers) ? computers.filter(isPlanComputer) : []
}

/** How long a checkout is watched before neo stops waiting for it. */
export const CHECKOUT_GIVE_UP_MS = 30 * 60_000

export type CheckoutPhase =
  | 'idle'
  | 'waiting'
  | 'confirming'
  | 'complete'
  | 'abandoned'

export interface CheckoutProgress {
  /** A checkout was started and not yet put away. */
  started: boolean
  /** The person said they would not pay. */
  cancelled: boolean
  status?: PlanCheckoutStatus
  startedAt: number
  /** When the poll last answered, with data or an error. */
  lastCheckedAt: number
}

export function checkoutPhase(progress: CheckoutProgress): CheckoutPhase {
  if (progress.cancelled) return 'abandoned'
  if (!progress.started) return 'idle'
  const status = progress.status
  if (status?.status === 'expired') return 'abandoned'
  if (status?.status === 'complete') {
    return status.plan ? 'complete' : 'confirming'
  }
  if (progress.lastCheckedAt - progress.startedAt > CHECKOUT_GIVE_UP_MS) {
    return 'abandoned'
  }
  return 'waiting'
}
