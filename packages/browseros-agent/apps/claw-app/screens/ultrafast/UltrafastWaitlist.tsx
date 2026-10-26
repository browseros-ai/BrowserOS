/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Waitlist for Ultrafast mode, used to measure demand at a monthly price.
 * Signups are the `ultrafast_waitlist_joined` event against
 * `ultrafast_waitlist_viewed`, split by the `price_usd` each reader was shown.
 *
 * Email is sent only on explicit submission, independently of usage analytics.
 * Confirmation waits for PostHog to accept the event. Usage views still respect
 * analytics consent, and signup never enables tracking or person profiles.
 */

import { Check } from 'lucide-react'
import {
  type FormEvent,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { Page } from '@/components/layout/Page'
import { PageHeader } from '@/components/layout/PageHeader'
import { AnimatedBadge } from '@/components/motion/animated-badge'
import { StatefulButton } from '@/components/motion/button/stateful'
import { Card } from '@/components/motion/card'
import { Input } from '@/components/motion/input'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import {
  isCapturing,
  subscribeToCaptureState,
} from '@/modules/analytics/posthog'
import {
  rememberJoined,
  type UltrafastPrice,
  useUltrafastJoined,
  useUltrafastPrice,
} from './ultrafast-price'
import { submitUltrafastSignup } from './ultrafast-signup'

const PERKS = [
  'Faster clicks, typing, and navigation',
  'Keep using your own Claude, Codex, or other AI subscription',
  'Your AI agents use far fewer tokens, because our model handles most browser actions',
]

const BACK_HOME = { to: '/', label: 'Home' }

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

/** Trimmed, lowercased email, or null when it does not look like one. */
export function normalizeEmail(value: string): string | null {
  const email = value.trim().toLowerCase()
  return EMAIL_PATTERN.test(email) ? email : null
}

export function UltrafastWaitlist() {
  const priceState = useUltrafastPrice()
  const joinedAnywhere = useUltrafastJoined()
  // Covers storage being unavailable, where the shared flag cannot be written.
  const [joinedHere, setJoinedHere] = useState(false)
  const joined = joinedAnywhere || joinedHere
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const joining = useRef(false)
  const capturing = useSyncExternalStore(
    subscribeToCaptureState,
    isCapturing,
    () => false,
  )
  const viewTracked = useRef(false)
  const price = priceState.status === 'ready' ? priceState.price : null

  // Waits for capture so a cold open still counts its view once PostHog is up.
  useEffect(() => {
    if (price === null || !capturing || viewTracked.current) return
    viewTracked.current = true
    track(AnalyticsEvent.UltrafastWaitlistViewed, {
      price_usd: price,
      already_joined: joined,
    })
  }, [capturing, joined, price])

  if (priceState.status !== 'ready') {
    return <UltrafastWaitlistPending status={priceState.status} />
  }

  const handleJoin = async (email: string) => {
    if (joined || joining.current) return
    joining.current = true
    setPending(true)
    setError(null)
    try {
      await submitUltrafastSignup({
        price: priceState.price,
        distinctId: priceState.distinctId,
        email,
      })
      rememberJoined()
      setJoinedHere(true)
    } catch {
      setError("Couldn't save your signup. Please try again.")
    } finally {
      joining.current = false
      setPending(false)
    }
  }

  return (
    <UltrafastWaitlistView
      price={priceState.price}
      joined={joined}
      pending={pending}
      error={error}
      onJoin={handleJoin}
    />
  )
}

function UltrafastWaitlistPending({
  status,
}: {
  status: 'loading' | 'unavailable'
}) {
  return (
    <Page>
      <PageHeader title="Ultrafast mode" back={BACK_HOME} />
      <p role="status" className="text-[15px] text-ink-2">
        {status === 'loading'
          ? 'Loading…'
          : "Can't reach neo's local service right now. This page will load once it's back."}
      </p>
    </Page>
  )
}

export function UltrafastWaitlistView({
  price,
  joined,
  pending = false,
  error = null,
  onJoin,
}: {
  price: UltrafastPrice
  joined: boolean
  pending?: boolean
  error?: string | null
  onJoin: (email: string) => void
}) {
  const [value, setValue] = useState('')
  const [invalid, setInvalid] = useState(false)

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (pending || joined) return
    const email = normalizeEmail(value)
    setInvalid(email === null)
    if (email) onJoin(email)
  }

  return (
    <Page>
      <div className="flex flex-col gap-3">
        <AnimatedBadge
          status="info"
          size="sm"
          showIcon={false}
          className="self-start"
        >
          Coming soon
        </AnimatedBadge>
        <PageHeader
          title="Ultrafast mode"
          lede="We trained our own model for browser actions, so tasks finish faster."
          back={BACK_HOME}
        />
      </div>

      <Card
        aria-label="Ultrafast pricing"
        className="gap-[18px] border-border-2 bg-card px-6 shadow-card"
      >
        <div className="flex flex-wrap items-baseline justify-between gap-3">
          <h2 className="font-bold text-[17px] text-cyanotype-ink tracking-[-0.01em]">
            Unlimited Ultrafast access
          </h2>
          <p className="flex items-baseline gap-1">
            <span className="font-extrabold text-[28px] text-cyanotype-ink tabular-nums tracking-[-0.03em]">
              ${price}
            </span>
            <span className="text-[14px] text-ink-3">/month</span>
          </p>
        </div>
        <ul className="flex flex-col gap-3">
          {PERKS.map((perk) => (
            <li
              key={perk}
              className="flex items-start gap-2.5 text-[15px] text-ink leading-[22px]"
            >
              <Check
                aria-hidden="true"
                className="mt-0.5 size-[17px] shrink-0 text-primary"
              />
              <span className="min-w-0 flex-1">{perk}</span>
            </li>
          ))}
        </ul>
        {joined ? (
          <p
            role="status"
            className="flex items-center gap-2 font-semibold text-[15px] text-green-ink"
          >
            <Check aria-hidden className="size-4" />
            You're on the list. We'll email you when it's ready.
          </p>
        ) : (
          <form
            onSubmit={handleSubmit}
            noValidate
            className="flex flex-col gap-2"
          >
            <div className="flex flex-col items-stretch gap-2.5 sm:flex-row sm:items-end">
              <Input
                type="email"
                name="email"
                label="Email"
                autoComplete="email"
                placeholder="you@example.com"
                value={value}
                disabled={pending}
                error={invalid ? 'Enter a valid email address.' : undefined}
                onChange={(next) => {
                  setValue(next)
                  setInvalid(false)
                }}
                className="min-w-0 sm:flex-1"
                classNames={{ field: 'bg-background' }}
              />
              <StatefulButton
                type="submit"
                state={pending ? 'loading' : 'idle'}
                loadingText="Joining…"
                className="h-11 font-semibold"
              >
                Join the waitlist
              </StatefulButton>
            </div>
            {error && (
              <p role="alert" className="text-[13px] text-destructive">
                {error}
              </p>
            )}
          </form>
        )}
      </Card>
    </Page>
  )
}
