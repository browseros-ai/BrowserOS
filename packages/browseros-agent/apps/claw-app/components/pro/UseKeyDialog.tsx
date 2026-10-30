/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { PlanKeyChange } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { BadgeCheck, CalendarX, X } from 'lucide-react'
import { type ReactNode, useRef, useState } from 'react'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { Input } from '@/components/motion/input'
import { MorphingModal } from '@/components/motion/morphing-modal'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { useActivateKey, usePlan, usePlanKey } from '@/modules/api/plan.hooks'
import { toastApiError } from '@/modules/toast/toast'
import {
  apiErrorCode,
  apiErrorMessage,
  formatDate,
  keyFullComputers,
  keyLooksWhole,
  normalizeKey,
} from '@/screens/pro/pro.helpers'
import { ComputerRow } from './ComputerRow'

const KEY_FIELD_ID = 'pro-licence-key'
const TOO_SHORT = 'Paste the whole key. It starts with NEO-.'
const FAILURE_LINES: Record<string, string> = {
  key_invalid:
    'That key didn’t work. Copy it again from your other computer and paste it here.',
  too_many_tries:
    'Too many tries from this computer. Wait a few minutes, then try again.',
}
const TRACKED_FAILURES = new Set([
  'key_invalid',
  'key_full',
  'too_many_tries',
  'already_subscribed',
])

interface UseKeyDialogProps {
  open: boolean
  onClose: () => void
  /** Starts a checkout on this computer's key, whose Pro has ended. */
  onGetProAgain: () => void
}

/**
 * Turns on Pro with a pasted key. The field is emptied after every attempt;
 * a full key keeps the paste only until one computer is picked to make room.
 * Mount it afresh for each opening.
 */
export function UseKeyDialog({
  open,
  onClose,
  onGetProAgain,
}: UseKeyDialogProps) {
  const queryClient = useQueryClient()
  const field = useRef<HTMLInputElement>(null)
  const [tooShort, setTooShort] = useState(false)
  const [lostOpen, setLostOpen] = useState(false)
  const [attempts, setAttempts] = useState(0)
  const activate = useActivateKey({
    onSuccess: (change) => {
      queryClient.setQueryData(usePlan.getKey(), change.plan)
      if (change.key) queryClient.setQueryData(usePlanKey.getKey(), change.key)
      track(AnalyticsEvent.ProKeyActivated, {
        plan_ended: change.plan.status === 'none',
      })
    },
    onError: (error) => {
      const code = apiErrorCode(error)
      if (code && TRACKED_FAILURES.has(code)) {
        track(AnalyticsEvent.ProKeyActivationFailed, { reason: code })
      }
      if (!apiErrorMessage(error)) toastApiError('Couldn’t turn on Pro', error)
    },
    onSettled: () => setAttempts((count) => count + 1),
  })

  const code = apiErrorCode(activate.error)
  const view = activate.data
    ? activate.data.plan.status === 'none'
      ? 'ended'
      : 'done'
    : code === 'key_full'
      ? 'full'
      : 'enter'
  const fullComputers = keyFullComputers(activate.error)
  const failure = tooShort
    ? TOO_SHORT
    : code
      ? (FAILURE_LINES[code] ?? apiErrorMessage(activate.error))
      : undefined

  const submit = () => {
    const value = field.current?.value ?? ''
    if (!keyLooksWhole(value)) {
      activate.reset()
      setTooShort(true)
      return
    }
    setTooShort(false)
    activate.mutate({ key: normalizeKey(value) })
  }

  return (
    <MorphingModal
      viewId={open ? view : null}
      onClose={onClose}
      placement="center"
      className="max-w-[460px] border-border-2 bg-card"
    >
      {view === 'done' && activate.data ? (
        <DoneView change={activate.data} onClose={onClose} />
      ) : view === 'ended' ? (
        <EndedView
          onClose={onClose}
          onGetProAgain={() => {
            onClose()
            onGetProAgain()
          }}
        />
      ) : view === 'full' ? (
        <DialogShell
          title={`This key is on ${fullComputers.length} computers`}
          onClose={onClose}
        >
          <p className="text-[14.5px] text-ink-2">
            Remove one to make room for this computer. The one you remove
            switches to Free the next time it checks in.
          </p>
          <ul className="flex flex-col divide-y divide-border-2">
            {fullComputers.map((computer) => (
              <ComputerRow
                key={computer.installId}
                computer={computer}
                now={activate.submittedAt}
                actions={
                  <Button
                    variant="secondary"
                    size="sm"
                    disabled={activate.isPending}
                    onClick={() => {
                      const pasted = activate.variables?.key
                      if (pasted) {
                        activate.mutate({
                          key: pasted,
                          replace: computer.installId,
                        })
                      }
                    }}
                  >
                    Remove
                  </Button>
                }
              />
            ))}
          </ul>
          <div className="flex justify-end">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
          </div>
        </DialogShell>
      ) : (
        <DialogShell title="Use a licence key" onClose={onClose}>
          <p className="text-[14.5px] text-ink-2">
            Have Pro on another computer, or reinstalled neo? Paste your licence
            key to turn on Pro here.
          </p>
          <form
            className="flex flex-col gap-1.5"
            onSubmit={(event) => {
              event.preventDefault()
              submit()
            }}
          >
            <label
              htmlFor={KEY_FIELD_ID}
              className="font-semibold text-[14px] text-cyanotype-ink"
            >
              Licence key
            </label>
            <Input
              key={attempts}
              ref={field}
              id={KEY_FIELD_ID}
              placeholder="NEO-XXXXX-XXXXX-XXXXX-XXXXX-XXXXX"
              autoComplete="off"
              autoCapitalize="characters"
              spellCheck={false}
              autoFocus
              disabled={activate.isPending}
              error={failure}
              classNames={{
                input: 'font-mono uppercase tracking-[0.02em]',
                errorMessage: 'text-[13px] text-red-ink',
              }}
            />
            {failure ? null : (
              <span className="text-[13px] text-ink-3">
                It’s in the email we sent when you paid.
              </span>
            )}
          </form>
          {lostOpen ? (
            <p className="text-[13.5px] text-ink-2">
              Search your email for “neo Pro licence key”. Still nothing? Open
              neo Pro on a computer that has Pro and reset the key, or contact
              BrowserOS support with your Stripe receipt.
            </p>
          ) : (
            <button
              type="button"
              onClick={() => setLostOpen(true)}
              className="w-fit font-semibold text-[13.5px] text-accent-ink hover:underline"
            >
              Can’t find your key?
            </button>
          )}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <StatefulButton
              state={activate.isPending ? 'loading' : 'idle'}
              loadingText="Turn on Pro"
              disabled={activate.isPending}
              onClick={submit}
              className="font-semibold"
            >
              Turn on Pro
            </StatefulButton>
          </div>
        </DialogShell>
      )}
    </MorphingModal>
  )
}

// ---------------------------------------------------------------------------

function DialogShell({
  title,
  onClose,
  children,
}: {
  title: string
  onClose: () => void
  children: ReactNode
}) {
  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start justify-between gap-3">
        <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
          {title}
        </h2>
        <Button
          variant="ghost"
          size="icon"
          aria-label="Close"
          onClick={onClose}
          className="-mt-1 -mr-1 shrink-0"
        >
          <X aria-hidden className="size-4" />
        </Button>
      </div>
      {children}
    </div>
  )
}

function DoneView({
  change,
  onClose,
}: {
  change: PlanKeyChange
  onClose: () => void
}) {
  const renews =
    change.plan.status === 'active' && change.plan.periodEnd !== null
      ? `It renews on ${formatDate(change.plan.periodEnd)}. `
      : ''
  return (
    <div className="flex flex-col items-center gap-5 text-center">
      <span
        aria-hidden
        className="grid size-14 place-items-center rounded-full bg-green-tint text-green-ink"
      >
        <BadgeCheck className="size-6" />
      </span>
      <div className="flex flex-col gap-2">
        <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
          Pro is on for this computer
        </h2>
        <p className="text-[14.5px] text-ink-2">
          {renews}
          {change.key
            ? `Your key is now on ${change.key.computers.length} of ${change.key.cap} computers.`
            : null}
        </p>
      </div>
      <Button onClick={onClose} className="font-semibold">
        Done
      </Button>
    </div>
  )
}

function EndedView({
  onClose,
  onGetProAgain,
}: {
  onClose: () => void
  onGetProAgain: () => void
}) {
  return (
    <div className="flex flex-col items-center gap-5 text-center">
      <span
        aria-hidden
        className="grid size-14 place-items-center rounded-full bg-amber-tint text-amber-ink"
      >
        <CalendarX className="size-6" />
      </span>
      <div className="flex flex-col gap-2">
        <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
          This key’s Pro has ended
        </h2>
        <p className="text-[14.5px] text-ink-2">
          Your key works, but the Pro plan it belongs to has ended. Get Pro
          again and it comes back here and on every computer that uses this key.
        </p>
        <p className="text-[13.5px] text-ink-3">
          Your key stays the same, so there’s nothing new to paste.
        </p>
      </div>
      <div className="flex justify-center gap-2">
        <Button variant="ghost" onClick={onClose}>
          Not now
        </Button>
        <Button onClick={onGetProAgain} className="font-semibold">
          Get Pro again
        </Button>
      </div>
    </div>
  )
}
