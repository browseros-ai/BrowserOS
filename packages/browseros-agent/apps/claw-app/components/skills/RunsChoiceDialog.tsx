/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { useQueryClient } from '@tanstack/react-query'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
} from '@/components/motion/center-morph-modal'
import {
  useRunWithAgent,
  useRunWithoutAgent,
  useSkill,
  useSkills,
} from '@/modules/api/skills.hooks'
import { toastApiError, toastSuccess } from '@/modules/toast/toast'

interface RunsChoiceDialogProps {
  name: string
  /** The option being switched to, or null while closed. */
  target: 'with_agent' | 'without_agent' | null
  click: string | undefined
  site: string | undefined
  fastLabel: string | null
  onClose: () => void
}

/** Confirms switching a write skill to run with or without the agent. */
export function RunsChoiceDialog({
  name,
  target,
  click,
  site,
  fastLabel,
  onClose,
}: RunsChoiceDialogProps) {
  const queryClient = useQueryClient()
  const settled = (message: string) => {
    void queryClient.invalidateQueries({ queryKey: useSkill.getKey({ name }) })
    void queryClient.invalidateQueries({ queryKey: useSkills.getKey() })
    toastSuccess(message)
    onClose()
  }
  const without = useRunWithoutAgent({
    onSuccess: () => settled(`/${name} now runs without your agent.`),
    onError: (error) => toastApiError('Couldn’t change how it runs', error),
  })
  const withAgent = useRunWithAgent({
    onSuccess: () => settled(`/${name} now runs with your agent.`),
    onError: (error) => toastApiError('Couldn’t change how it runs', error),
  })
  const pending = without.isPending || withAgent.isPending
  const toWithout = target === 'without_agent'
  const where = site ? ` on ${site}` : ''

  return (
    <CenterMorphModal
      open={target !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
    >
      <CenterMorphModalContent
        ariaLabel={
          toWithout
            ? `Run /${name} without your agent?`
            : `Run /${name} with your agent?`
        }
        showCloseButton={false}
        className="max-w-[460px] border-border-2 bg-card"
      >
        <div className="flex flex-col gap-5 p-6">
          <div className="flex flex-col gap-2">
            <h2 className="font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
              {toWithout
                ? `Run /${name} without your agent?`
                : `Run /${name} with your agent?`}
            </h2>
            <p className="text-[14.5px] text-ink-2">
              {toWithout
                ? `neo will do it by itself${fastLabel ? ` in ${fastLabel}` : ''}, including this step${where}: “${click ?? 'its last step'}”. It only stops if a step can’t find what it expects, so it won’t notice things like the wrong item. You can switch back anytime.`
                : 'Your agent will read each page as it goes. It’s slower, but safer. You can switch back anytime.'}
            </p>
          </div>
          <div className="flex justify-end gap-2">
            <CenterMorphModalClose>
              <Button variant="ghost" disabled={pending}>
                Cancel
              </Button>
            </CenterMorphModalClose>
            <StatefulButton
              state={pending ? 'loading' : 'idle'}
              loadingText="Saving"
              disabled={pending}
              onClick={() =>
                toWithout
                  ? without.mutate({ name })
                  : withAgent.mutate({ name })
              }
              className="font-semibold"
            >
              {toWithout ? 'Run without my agent' : 'Run with my agent'}
            </StatefulButton>
          </div>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}
