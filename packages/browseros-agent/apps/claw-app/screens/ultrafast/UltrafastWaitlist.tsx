/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Waitlist for Ultrafast mode, used to measure demand at a monthly price.
 * Signups are the `ultrafast_waitlist_joined` event against
 * `ultrafast_waitlist_viewed`, split by the `price_usd` each reader was shown.
 *
 * The email the reader types is the one piece of user content the cockpit
 * sends, and only because they submitted it to join. It rides on the join
 * event rather than `identify()`: identity must stay the server's anonymous
 * UUID, and person profiles are off, so the event is where it is stored.
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
  readJoined,
  rememberJoined,
  type UltrafastPrice,
  useUltrafastPrice,
} from './ultrafast-price'

const PERKS = [
  'Agents finish browser tasks in a fraction of the time',
  'Early access to new speed features',
]

const EMAIL_PATTERN = /^[^\s@]+@[^\s@]+\.[^\s@]+$/

/** Trimmed, lowercased email, or null when it does not look like one. */
export function normalizeEmail(value: string): string | null {
  const email = value.trim().toLowerCase()
  return EMAIL_PATTERN.test(email) ? email : null
}

export function UltrafastWaitlist() {
  const price = useUltrafastPrice()
  const [joined, setJoined] = useState(readJoined)
  const capturing = useSyncExternalStore(
    subscribeToCaptureState,
    isCapturing,
    () => false,
  )
  const viewTracked = useRef(false)

  // Waits for capture so a cold open still counts its view once PostHog is up.
  useEffect(() => {
    if (price === null || !capturing || viewTracked.current) return
    viewTracked.current = true
    track(AnalyticsEvent.UltrafastWaitlistViewed, {
      price_usd: price,
      already_joined: joined,
    })
  }, [capturing, joined, price])

  if (price === null) return null

  const handleJoin = (email: string) => {
    track(AnalyticsEvent.UltrafastWaitlistJoined, { price_usd: price, email })
    rememberJoined()
    setJoined(true)
  }

  return (
    <UltrafastWaitlistView
      price={price}
      joined={joined}
      canJoin={capturing}
      onJoin={handleJoin}
    />
  )
}

export function UltrafastWaitlistView({
  price,
  joined,
  canJoin,
  onJoin,
}: {
  price: UltrafastPrice
  joined: boolean
  /** False when analytics is off, since the signup could not be saved. */
  canJoin: boolean
  onJoin: (email: string) => void
}) {
  const [value, setValue] = useState('')
  const [invalid, setInvalid] = useState(false)

  const handleSubmit = (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const email = normalizeEmail(value)
    setInvalid(email === null)
    if (email) onJoin(email)
  }

  return (
    <div className="mx-auto flex w-full max-w-xl flex-col gap-8 px-8 pt-16 pb-16">
      <header className="space-y-3">
        <p className="font-semibold text-[12px] text-cyanotype-blue uppercase tracking-[0.08em]">
          Coming soon
        </p>
        <h1 className="font-extrabold text-[28px] text-cyanotype-ink leading-[1.15] tracking-[-0.025em]">
          <span className="font-bold text-cyanotype-blue italic">
            Ultrafast
          </span>{' '}
          mode
        </h1>
        <p className="text-[14px] text-cyanotype-soft leading-6">
          The same agents, a lot less waiting. Join the waitlist and we'll turn
          it on for you first.
        </p>
      </header>

      <section
        aria-label="Ultrafast pricing"
        className="rounded-[9px] border border-cyanotype-border bg-accent-tint/50 p-6"
      >
        <div className="flex items-center gap-2 text-cyanotype-blue">
          <Zap className="size-4" />
          <span className="font-semibold text-[13px]">Ultrafast</span>
        </div>
        <p className="mt-3 flex items-baseline gap-1 text-cyanotype-ink">
          <span className="font-extrabold text-[40px] tabular-nums leading-none tracking-[-0.03em]">
            ${price}
          </span>
          <span className="text-[14px] text-cyanotype-soft">/month</span>
        </p>
        <ul className="mt-5 space-y-2">
          {PERKS.map((perk) => (
            <li
              key={perk}
              className="flex items-start gap-2 text-[13px] text-cyanotype-ink leading-5"
            >
              <Check className="mt-0.5 size-4 shrink-0 text-cyanotype-blue" />
              {perk}
            </li>
          ))}
        </ul>
        <div className="mt-6">
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
              <div className="flex gap-2">
                <Input
                  type="email"
                  name="email"
                  autoComplete="email"
                  placeholder="you@example.com"
                  aria-label="Email"
                  aria-invalid={invalid || undefined}
                  value={value}
                  disabled={!canJoin}
                  onChange={(event) => {
                    setValue(event.target.value)
                    setInvalid(false)
                  }}
                  className="h-9 flex-1 bg-background text-[13px]"
                />
                <Button
                  type="submit"
                  disabled={!canJoin}
                  className="h-9 shrink-0 bg-cyanotype-blue px-4 text-[13px] text-on-cyanotype hover:bg-cyanotype-blue-hover"
                >
                  Join the waitlist
                </Button>
              </div>
              {invalid && (
                <p role="alert" className="text-[12px] text-destructive">
                  Enter a valid email address.
                </p>
              )}
              {!canJoin && (
                <p className="text-[12px] text-cyanotype-soft">
                  Signups are saved through usage analytics, which is off. Turn
                  it on under Privacy in the sidebar to join.
                </p>
              )}
            </form>
          )}
          <p className="mt-3 text-[12px] text-cyanotype-muted">
            No payment today. You'll choose whether to subscribe at launch.
          </p>
        </div>
      </section>
    </div>
  )
}
