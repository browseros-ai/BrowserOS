/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { PlanComputer, PlanKey } from '@browseros/claw-api'
import { Pencil, RotateCcwKey } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/motion/button/base'
import { Skeleton } from '@/components/ui/skeleton'
import {
  apiErrorMessage,
  computerName,
  keyHint,
  PLAN_UNCHECKED,
} from '@/screens/pro/pro.helpers'
import { ComputerRow, RenameComputerRow } from './ComputerRow'
import { RemoveComputerDialog } from './RemoveComputerDialog'
import { ResetKeyDialog } from './ResetKeyDialog'

interface KeySectionProps {
  /** The `usePlanKey` result for this computer's key. */
  planKey: {
    data?: PlanKey
    isPending: boolean
    isError: boolean
    error: unknown
    dataUpdatedAt: number
  }
}

/**
 * Where the key is (the email, never this page), the computers using it, and
 * Reset key, with no box around them. neo never has the key itself, only its
 * first group.
 */
export function KeySection({ planKey }: KeySectionProps) {
  const [renaming, setRenaming] = useState<string | null>(null)
  const [removing, setRemoving] = useState<PlanComputer | null>(null)
  const [removeOpen, setRemoveOpen] = useState(false)
  const [resetOpen, setResetOpen] = useState(false)
  const [resetOpens, setResetOpens] = useState(0)
  const key = planKey.data

  return (
    <section aria-label="Your licence key" className="flex flex-col gap-3">
      <div className="flex items-center justify-between gap-3">
        <h2 className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]">
          Your licence key
        </h2>
        <Button
          variant="ghost"
          size="sm"
          onClick={() => {
            setResetOpens((count) => count + 1)
            setResetOpen(true)
          }}
          className="font-semibold text-ink-2"
        >
          <RotateCcwKey aria-hidden className="size-4" />
          Reset key
        </Button>
      </div>
      <p className="text-[14.5px] text-ink-2">
        It’s in the email we sent when you paid
        {key ? (
          <>
            {' '}
            and starts{' '}
            <span className="font-mono text-[13.5px] text-cyanotype-ink">
              {keyHint(key.hint)}
            </span>
          </>
        ) : null}
        . Keep it private.
      </p>

      <div className="mt-4 flex items-baseline justify-between gap-3 font-bold text-[15px] text-cyanotype-ink">
        <span>Computers</span>
        {key ? (
          <span className="whitespace-nowrap font-semibold text-[13.5px] text-ink-3 tabular-nums">
            {key.computers.length} of {key.cap}
          </span>
        ) : null}
      </div>
      {planKey.isPending ? (
        <div className="flex flex-col gap-3">
          <Skeleton className="h-11 w-full" />
          <Skeleton className="h-11 w-full" />
        </div>
      ) : planKey.isError || !key ? (
        <p role="alert" className="text-[14px] text-ink-2">
          {apiErrorMessage(planKey.error) ?? PLAN_UNCHECKED}
        </p>
      ) : (
        <ul className="flex flex-col divide-y divide-border-2 border-border-2 border-y">
          {key.computers.map((computer) =>
            renaming === computer.installId ? (
              <RenameComputerRow
                key={computer.installId}
                computer={computer}
                onDone={() => setRenaming(null)}
              />
            ) : (
              <ComputerRow
                key={computer.installId}
                computer={computer}
                now={planKey.dataUpdatedAt}
                brief
                actions={
                  <>
                    {computer.thisComputer ? (
                      <span className="whitespace-nowrap rounded-full bg-green-tint px-2.5 font-semibold text-[12.5px] text-green-ink leading-[22px]">
                        This computer
                      </span>
                    ) : (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => {
                          setRemoving(computer)
                          setRemoveOpen(true)
                        }}
                      >
                        Remove
                      </Button>
                    )}
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`Rename ${computerName(computer)}`}
                      title="Rename"
                      onClick={() => setRenaming(computer.installId)}
                      className="text-ink-3 hover:text-cyanotype-ink"
                    >
                      <Pencil aria-hidden className="size-4" />
                    </Button>
                  </>
                }
              />
            ),
          )}
        </ul>
      )}

      <RemoveComputerDialog
        open={removeOpen}
        computer={removing}
        onClose={() => setRemoveOpen(false)}
      />
      <ResetKeyDialog
        key={resetOpens}
        open={resetOpen}
        onOpenChange={setResetOpen}
      />
    </section>
  )
}
