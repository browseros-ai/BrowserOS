/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { PlanComputer } from '@browseros/claw-api'
import { useQueryClient } from '@tanstack/react-query'
import { Laptop, Monitor } from 'lucide-react'
import { type ReactNode, useRef, useState } from 'react'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { Input } from '@/components/motion/input'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { usePlanKey, useRenameComputer } from '@/modules/api/plan.hooks'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'
import {
  COMPUTER_NAME_MAX,
  computerName,
  formatAdded,
  lastUsedLine,
} from '@/screens/pro/pro.helpers'

interface ComputerRowProps {
  computer: PlanComputer
  /** The reference time for "Last used", so a render stays pure. */
  now: number
  /** Buttons at the end of the row, in one aligned column. */
  actions: ReactNode
}

/** One computer on the key: its name, when it was added and last used. */
export function ComputerRow({ computer, now, actions }: ComputerRowProps) {
  return (
    <li className="grid grid-cols-[36px_minmax(0,1fr)_auto] items-center gap-3 py-2.5">
      <ComputerIcon platform={computer.platform} />
      <div className="min-w-0">
        <div className="truncate font-semibold text-[14.5px] text-cyanotype-ink">
          {computerName(computer)}
        </div>
        <div className="text-[13px] text-ink-3">
          <span className="whitespace-nowrap">
            Added {formatAdded(computer.activatedAt)}
          </span>
          {' · '}
          {computer.thisComputer
            ? 'You’re using it now'
            : lastUsedLine(computer.lastSeenAt, now)}
        </div>
      </div>
      <span className="flex items-center gap-1">{actions}</span>
    </li>
  )
}

interface RenameComputerRowProps {
  computer: PlanComputer
  onDone: () => void
}

/** The row as a name field: Enter saves, Escape cancels, the typed name survives a failure. */
export function RenameComputerRow({
  computer,
  onDone,
}: RenameComputerRowProps) {
  const field = useRef<HTMLInputElement>(null)
  const [empty, setEmpty] = useState(false)
  const queryClient = useQueryClient()
  const rename = useRenameComputer({
    onSuccess: (key, { name }) => {
      queryClient.setQueryData(usePlanKey.getKey(), key)
      toastSuccess(`Renamed to ${name}`)
      track(AnalyticsEvent.ProComputerRenamed)
      onDone()
    },
    onError: (error) => toastApiError('Couldn’t rename this computer', error),
  })

  const save = () => {
    const name = (field.current?.value ?? '').trim().slice(0, COMPUTER_NAME_MAX)
    if (!name) {
      setEmpty(true)
      return
    }
    setEmpty(false)
    rename.mutate({ installId: computer.installId, name })
  }

  return (
    <li className="grid grid-cols-[36px_minmax(0,1fr)_auto] items-start gap-3 py-2.5">
      <ComputerIcon platform={computer.platform} className="mt-0.5" />
      <form
        className="flex min-w-0 flex-col gap-1.5"
        onSubmit={(event) => {
          event.preventDefault()
          save()
        }}
      >
        <Input
          ref={field}
          aria-label="Name for this computer"
          defaultValue={computerName(computer)}
          maxLength={COMPUTER_NAME_MAX}
          autoComplete="off"
          spellCheck={false}
          autoFocus
          disabled={rename.isPending}
          error={empty ? 'Give this computer a name.' : undefined}
          onKeyDown={(event) => {
            if (event.key === 'Escape') onDone()
          }}
          classNames={{
            field: 'h-[38px]',
            input: 'font-semibold text-[14.5px]',
            errorMessage: 'text-[13px] text-red-ink',
          }}
        />
        {empty ? null : (
          <span className="text-[13px] text-ink-3">
            Added {formatAdded(computer.activatedAt)}
          </span>
        )}
      </form>
      <span className="flex items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          disabled={rename.isPending}
          onClick={onDone}
        >
          Cancel
        </Button>
        <StatefulButton
          size="sm"
          state={rename.isPending ? 'loading' : 'idle'}
          loadingText="Save"
          disabled={rename.isPending}
          onClick={save}
          className="font-semibold"
        >
          Save
        </StatefulButton>
      </span>
    </li>
  )
}

// ---------------------------------------------------------------------------

function ComputerIcon({
  platform,
  className,
}: {
  platform: string
  className?: string
}) {
  const Icon = platform === 'macos' ? Laptop : Monitor
  return (
    <span
      aria-hidden
      className={cn(
        'grid size-9 place-items-center rounded-[10px] bg-accent-tint text-accent-ink',
        className,
      )}
    >
      <Icon className="size-4" />
    </span>
  )
}
