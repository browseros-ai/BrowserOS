/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { PlanComputer, PlanKey } from '@browseros/claw-api'
import { Pencil, RotateCcwKey } from 'lucide-react'
import { useState } from 'react'
import { Button } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import { Skeleton } from '@/components/ui/skeleton'
import {
  apiErrorMessage,
  computerName,
  keyHint,
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
 * Reset key. neo never has the key itself, only its first group.
 */
export function KeySection({ planKey }: KeySectionProps) {
  const [renaming, setRenaming] = useState<string | null>(null)
  const [removing, setRemoving] = useState<PlanComputer | null>(null)
  const [removeOpen, setRemoveOpen] = useState(false)
  const [resetOpen, setResetOpen] = useState(false)
  const [resetOpens, setResetOpens] = useState(0)
  const key = planKey.data

  return (
    <section aria-label="Your licence key" className="flex flex-col gap-3.5">
      <h2 className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]">
        Your licence key
      </h2>
      <Card
        size="compact"
        className="gap-0 border-border-2 bg-card py-0 shadow-card"
      >
        <div className="flex flex-col gap-3 px-[22px] py-5">
          <p className="text-[14.5px] text-ink-2">
            It’s in the email we sent when you paid. Paste it in neo on another
            computer to use Pro there. Anyone with it can use your Pro, so keep
            it to yourself.
            {key ? (
              <>
                {' '}
                It starts{' '}
                <span className="font-mono text-[13.5px] text-cyanotype-ink">
                  {keyHint(key.hint)}
                </span>
                .
              </>
            ) : null}
          </p>
          <p className="text-[13.5px] text-ink-3">
            Can’t find the email? Reset your key and we’ll email you a new one.
          </p>
        </div>

        <div className="flex flex-col gap-3 border-border-2 border-t px-[22px] py-5">
          <div className="flex items-baseline justify-between gap-3 font-bold text-[15.5px] text-cyanotype-ink">
            <span>Computers using Pro</span>
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
              {apiErrorMessage(planKey.error) ??
                'Couldn’t check your plan. Connect to the internet and neo checks again.'}
            </p>
          ) : (
            <ul className="flex flex-col divide-y divide-border-2">
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
        </div>

        <div className="flex flex-wrap items-center justify-between gap-3 border-border-2 border-t px-[22px] py-5 text-[14px] text-ink-2">
          <span>Shared your key by mistake, or lost a computer?</span>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => {
              setResetOpens((count) => count + 1)
              setResetOpen(true)
            }}
            className="font-semibold"
          >
            <RotateCcwKey aria-hidden className="size-4" />
            Reset key
          </Button>
        </div>
      </Card>

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
