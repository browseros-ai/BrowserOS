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

import { Check, Zap } from 'lucide-react'
import {
  type FormEvent,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
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
  'Bring your own Claude, Codex, or other agent subscription to power your main agent',
  'Use 10× fewer tokens from your main agent, with our browser-action model handling most actions',
]

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
    <div className="mx-auto flex w-full max-w-xl flex-col gap-3 px-8 py-14">
      <h1 className="font-[650] text-[32px] text-cyanotype-ink leading-[39px] tracking-[-0.035em]">
        <span className="text-cyanotype-blue italic">Ultrafast</span> mode
      </h1>
      <p role="status" className="text-[14px] text-cyanotype-soft leading-6">
        {status === 'loading'
          ? 'Loading…'
          : "Can't reach neo's local service right now. This page will load once it's back."}
      </p>
    </div>
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
    <div className="mx-auto flex w-full max-w-xl flex-col gap-7 px-8 py-14">
      <header className="space-y-3">
        <p className="font-semibold text-[12px] text-cyanotype-blue uppercase leading-[18px] tracking-[0.1em]">
          Coming soon
        </p>
        <h1 className="font-[650] text-[32px] text-cyanotype-ink leading-[39px] tracking-[-0.035em]">
          <span className="text-cyanotype-blue italic">Ultrafast</span> mode
        </h1>
        <p className="text-[15px] text-cyanotype-soft leading-6">
          We trained our own browser-action model for faster browser tasks.
        </p>
      </header>

      <section
        aria-label="Ultrafast pricing"
        className="flex flex-col gap-[22px] rounded-xl border border-cyanotype-border bg-card p-6 shadow-[0_3px_12px_#102a4305]"
      >
        <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 flex-1 items-center gap-3">
            <span className="flex size-10 shrink-0 items-center justify-center rounded-[10px] bg-accent-tint/50">
              <Zap
                aria-hidden="true"
                className="size-[22px] text-cyanotype-blue"
              />
            </span>
            <h2 className="min-w-0 flex-1 font-semibold text-[15px] text-cyanotype-ink leading-[22px] tracking-[-0.015em]">
              Unlimited browser-action model access
            </h2>
          </div>
          <p className="flex shrink-0 items-baseline gap-[3px] text-cyanotype-ink">
            <span className="font-bold text-[28px] tabular-nums leading-[34px] tracking-[-0.04em]">
              ${price}
            </span>
            <span className="text-[13px] text-cyanotype-soft">/month</span>
          </p>
        </div>
        <ul className="space-y-3.5">
          {PERKS.map((perk) => (
            <li
              key={perk}
              className="flex items-start gap-2.5 text-[14px] text-cyanotype-ink leading-[22px]"
            >
              <Check
                aria-hidden="true"
                className="mt-0.5 size-[18px] shrink-0 text-cyanotype-blue"
              />
              <span className="min-w-0 flex-1">{perk}</span>
            </li>
          ))}
        </ul>
        <div className="pt-0.5">
          {joined ? (
            <p
              role="status"
              className="flex items-center gap-2 font-semibold text-[13px] text-cyanotype-ink"
            >
              <Check className="size-4 text-cyanotype-blue" />
              You're on the list. We'll email you when it's ready.
            </p>
          ) : (
            <form onSubmit={handleSubmit} noValidate className="space-y-2">
              <div className="flex flex-col items-start gap-2 sm:flex-row">
                <Input
                  type="email"
                  name="email"
                  autoComplete="email"
                  placeholder="you@example.com"
                  aria-label="Email"
                  aria-invalid={invalid || undefined}
                  value={value}
                  disabled={pending}
                  onChange={(event) => {
                    setValue(event.target.value)
                    setInvalid(false)
                  }}
                  className="h-[42px] min-w-0 rounded-md border-cyanotype-border bg-background text-[14px] sm:flex-1"
                />
                <div className="flex w-full shrink-0 flex-col gap-2.5 sm:w-auto">
                  <Button
                    type="submit"
                    disabled={pending}
                    className="h-[42px] rounded-md bg-cyanotype-blue px-4 text-[14px] text-on-cyanotype hover:bg-cyanotype-blue-hover"
                  >
                    {pending ? 'Joining…' : 'Request early access'}
                  </Button>
                  <p className="text-center text-[12px] text-cyanotype-muted leading-[19px]">
                    No payment today.
                  </p>
                </div>
              </div>
              {invalid && (
                <p role="alert" className="text-[12px] text-destructive">
                  Enter a valid email address.
                </p>
              )}
              {error && (
                <p role="alert" className="text-[12px] text-destructive">
                  {error}
                </p>
              )}
            </form>
          )}
        </div>
      </section>
    </div>
  )
}
