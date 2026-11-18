/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SkillDraft, SkillDraftStatus } from '@browseros/claw-api'
import { Bookmark, Info, PenLine, Plus, TriangleAlert, X } from 'lucide-react'
import { useState } from 'react'
import { NavLink } from 'react-router'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import { CopyButton } from '@/components/copy/CopyButton'
import { Favicon } from '@/components/favicon/Favicon'
import { Button } from '@/components/motion/button/base'
import { StatefulButton } from '@/components/motion/button/stateful'
import { MorphingModal } from '@/components/motion/morphing-modal'
import { Phrase } from '@/components/skills/Phrase'
import { Input } from '@/components/ui/input'
import { Skeleton } from '@/components/ui/skeleton'
import { Textarea } from '@/components/ui/textarea'
import {
  neoName,
  SAVE_PHRASE,
  skillCommand,
} from '@/screens/skills/skills.helpers'
import {
  canSave,
  type DraftEdits,
  draftProblem,
  editsFrom,
} from './skill-draft.helpers'

interface SkillDraftSheetProps {
  open: boolean
  status: SkillDraftStatus | undefined
  /** "From your Flipkart task, today at 18:39". */
  source: string
  site?: string
  saving: boolean
  onClose: () => void
  onRetry: () => void
  onSave: (edits: DraftEdits) => void
}

/**
 * The sheet a draft is written into, read and saved from. It opens straight away in the shape of
 * the review, so nothing jumps when the draft arrives; closing it keeps the draft for 24 hours.
 */
export function SkillDraftSheet({
  open,
  status,
  source,
  site,
  saving,
  onClose,
  onRetry,
  onSave,
}: SkillDraftSheetProps) {
  const view =
    status?.status === 'ready' && status.draft
      ? 'review'
      : status?.status === 'failed'
        ? 'problem'
        : 'writing'
  return (
    <MorphingModal
      viewId={open ? view : null}
      onClose={onClose}
      placement="center"
      className="max-h-[calc(100dvh-32px)] max-w-[620px] overflow-y-auto border-border-2 bg-card"
    >
      <SheetHeader source={source} site={site} onClose={onClose} />
      {view === 'review' && status?.draft ? (
        <DraftReview
          key={status.draftId}
          draft={status.draft}
          saving={saving}
          onClose={onClose}
          onSave={onSave}
        />
      ) : view === 'problem' ? (
        <DraftProblemView status={status} onClose={onClose} onRetry={onRetry} />
      ) : (
        <DraftWriting onClose={onClose} />
      )}
    </MorphingModal>
  )
}

// ---------------------------------------------------------------------------

function SheetHeader({
  source,
  site,
  onClose,
}: {
  source: string
  site?: string
  onClose: () => void
}) {
  return (
    <div className="mb-4 flex items-start justify-between gap-3 border-border-2 border-b pb-3.5">
      <div className="flex min-w-0 flex-col gap-1">
        <h2 className="font-bold text-[21px] text-cyanotype-ink tracking-[-0.015em]">
          Make it a skill
        </h2>
        <p className="flex items-center gap-2 text-[14px] text-ink-3">
          {site ? (
            <span className="grid size-[22px] place-items-center rounded-md bg-accent-tint">
              <Favicon
                site={site}
                size={16}
                className="size-3.5 rounded-[3px]"
              />
            </span>
          ) : null}
          {source}
        </p>
      </div>
      <Button
        variant="ghost"
        size="icon"
        aria-label="Close"
        onClick={onClose}
        className="text-ink-3"
      >
        <X aria-hidden className="size-4" />
      </Button>
    </div>
  )
}

function Field({
  label,
  htmlFor,
  children,
  help,
}: {
  label: string
  htmlFor?: string
  children: React.ReactNode
  help?: React.ReactNode
}) {
  return (
    <div className="flex flex-col gap-[7px]">
      {htmlFor ? (
        <label
          htmlFor={htmlFor}
          className="font-semibold text-[13.5px] text-ink-2"
        >
          {label}
        </label>
      ) : (
        <span className="font-semibold text-[13.5px] text-ink-2">{label}</span>
      )}
      {children}
      {help ? <span className="text-[13px] text-ink-3">{help}</span> : null}
    </div>
  )
}

function SheetFooter({
  note,
  children,
}: {
  note?: string
  children: React.ReactNode
}) {
  return (
    <div className="mt-5 flex flex-wrap items-center justify-between gap-2.5 border-border-2 border-t pt-3.5">
      {note ? <span className="text-[13.5px] text-ink-3">{note}</span> : null}
      <div className="ml-auto flex gap-2">{children}</div>
    </div>
  )
}

/** About 10 seconds: a skeleton in the review's own shape and one line of copy. */
function DraftWriting({ onClose }: { onClose: () => void }) {
  return (
    <div className="flex flex-col gap-[18px]" aria-busy="true">
      <p className="flex items-center gap-2.5 text-[14.5px] text-ink-2">
        <PenLine aria-hidden className="size-4" />
        Writing your skill from this task. It takes about 10 seconds.
      </p>
      <div
        aria-hidden
        className="relative h-[3px] overflow-hidden rounded-full bg-bg-sunken"
      >
        <span className="absolute inset-y-0 left-0 w-[38%] rounded-full bg-primary motion-safe:animate-pulse" />
      </div>
      <Field label="Name">
        <Skeleton className="h-10" />
      </Field>
      <Field label="What it does">
        <Skeleton className="h-16" />
      </Field>
      <Field label="Steps">
        <div className="flex flex-col gap-2">
          <Skeleton className="h-[34px]" />
          <Skeleton className="h-[34px]" />
          <Skeleton className="h-[34px]" />
          <Skeleton className="h-[34px] w-[72%]" />
        </div>
      </Field>
      <SheetFooter note="Close this and your draft is kept for 24 hours.">
        <Button variant="secondary" onClick={onClose} className="font-semibold">
          Cancel
        </Button>
      </SheetFooter>
    </div>
  )
}

/** Everything in the draft is editable; nothing is saved until Save skill. */
function DraftReview({
  draft,
  saving,
  onClose,
  onSave,
}: {
  draft: SkillDraft
  saving: boolean
  onClose: () => void
  onSave: (edits: DraftEdits) => void
}) {
  // Seeded once from the draft; the sheet remounts this per draft id, so no effect syncs it.
  const [edits, setEdits] = useState<DraftEdits>(() => editsFrom(draft))
  const setStep = (index: number, value: string) =>
    setEdits((current) => ({
      ...current,
      steps: current.steps.map((step, at) => (at === index ? value : step)),
    }))
  const removeStep = (index: number) =>
    setEdits((current) => ({
      ...current,
      steps: current.steps.filter((_, at) => at !== index),
    }))
  const removeInput = (name: string) =>
    setEdits((current) => ({
      ...current,
      inputs: current.inputs.filter((input) => input.name !== name),
    }))

  return (
    <form
      className="flex flex-col gap-[18px]"
      onSubmit={(event) => {
        event.preventDefault()
        if (canSave(edits) && !saving) onSave(edits)
      }}
    >
      {draft.confidence === 'low' ? (
        <p className="flex gap-2.5 rounded-xl bg-amber-tint px-3.5 py-3 text-[14px] text-amber-ink">
          <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
          Check the steps before you save. This task took some turns, so the
          draft may need edits.
        </p>
      ) : null}
      <Field
        label="Name"
        htmlFor="skill-draft-name"
        help={
          <>
            Your agent runs it with{' '}
            <Phrase>{skillCommand(neoName(draft.name))}</Phrase>
          </>
        }
      >
        <Input
          id="skill-draft-name"
          value={edits.title}
          onChange={(event) =>
            setEdits((current) => ({ ...current, title: event.target.value }))
          }
          className="h-10 text-[15px]"
        />
      </Field>
      <Field label="What it does" htmlFor="skill-draft-description">
        <Textarea
          id="skill-draft-description"
          value={edits.description}
          onChange={(event) =>
            setEdits((current) => ({
              ...current,
              description: event.target.value,
            }))
          }
          className="resize-none text-[15px]"
        />
      </Field>
      <Field label="Steps">
        <ol className="flex flex-col gap-2">
          {edits.steps.map((step, index) => (
            <li
              // Steps have no ids, and their order is what the person edits.
              // biome-ignore lint/suspicious/noArrayIndexKey: position is the identity here
              key={index}
              className="grid grid-cols-[26px_minmax(0,1fr)_28px] items-center gap-2"
            >
              <span className="grid size-[26px] place-items-center rounded-full bg-accent-tint font-semibold text-[12.5px] text-accent-ink">
                {index + 1}
              </span>
              <Textarea
                rows={1}
                aria-label={`Step ${index + 1}`}
                value={step}
                onChange={(event) => setStep(index, event.target.value)}
                className="min-h-0 resize-none text-[14.5px]"
              />
              <Button
                type="button"
                variant="ghost"
                size="icon"
                aria-label={`Remove step ${index + 1}`}
                onClick={() => removeStep(index)}
                className="size-7 text-ink-3"
              >
                <X aria-hidden className="size-[15px]" />
              </Button>
            </li>
          ))}
        </ol>
        <button
          type="button"
          onClick={() =>
            setEdits((current) => ({
              ...current,
              steps: [...current.steps, ''],
            }))
          }
          className="inline-flex items-center gap-1.5 self-start font-semibold text-[14px] text-primary"
        >
          <Plus aria-hidden className="size-4" />
          Add a step
        </button>
      </Field>
      {edits.inputs.length > 0 ? (
        <Field label="What changes each time">
          <ul className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
            {edits.inputs.map((input) => (
              <li
                key={input.name}
                className="flex flex-col gap-1 rounded-xl border border-border-2 bg-card px-3.5 py-2.5"
              >
                <span className="flex items-center justify-between gap-2">
                  <span className="rounded-md bg-accent-tint px-1.5 font-mono text-[12.5px] text-accent-ink">
                    {input.name}
                  </span>
                  <Button
                    type="button"
                    variant="ghost"
                    size="icon"
                    aria-label={`Remove ${input.name}`}
                    onClick={() => removeInput(input.name)}
                    className="size-6 text-ink-3"
                  >
                    <X aria-hidden className="size-[14px]" />
                  </Button>
                </span>
                <span className="text-[14px] text-ink-2">
                  Example{' '}
                  <b className="font-semibold text-cyanotype-ink">
                    {input.example}
                  </b>
                </span>
              </li>
            ))}
          </ul>
        </Field>
      ) : null}
      {draft.effects === 'write' ? (
        <p className="flex gap-2.5 rounded-xl bg-amber-tint px-3.5 py-3 text-[14px] text-amber-ink">
          <Info aria-hidden className="mt-0.5 size-4 shrink-0" />
          This skill changes something on the site. Your agent will ask before
          it clicks.
        </p>
      ) : null}
      <p className="flex gap-2.5 rounded-xl bg-accent-tint px-3.5 py-3 text-[14px] text-ink">
        <Info aria-hidden className="mt-0.5 size-4 shrink-0 text-accent-ink" />
        <span>
          It starts as a regular skill. After your agent runs it once, it
          becomes <UltrafastWordmark className="text-[15px]" />.
        </span>
      </p>
      <SheetFooter note="You can change it later on the Skills page.">
        <Button
          type="button"
          variant="secondary"
          onClick={onClose}
          className="font-semibold"
        >
          Cancel
        </Button>
        <StatefulButton
          type="submit"
          state={saving ? 'loading' : 'idle'}
          loadingText="Saving…"
          disabled={saving || !canSave(edits)}
          aria-disabled={saving || !canSave(edits)}
          className="font-semibold"
        >
          Save skill
        </StatefulButton>
      </SheetFooter>
    </form>
  )
}

/** A limit, a pause or a failure, always with the route through the person's own agent. */
function DraftProblemView({
  status,
  onClose,
  onRetry,
}: {
  status: SkillDraftStatus | undefined
  onClose: () => void
  onRetry: () => void
}) {
  const problem = draftProblem(status?.failure)
  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-2">
        <h3 className="font-bold text-[18px] text-cyanotype-ink">
          {problem.title}
        </h3>
        <p className="max-w-[52ch] text-ink-2">{problem.detail}</p>
      </div>
      <div className="flex flex-col gap-2 border-border-2 border-t pt-3.5">
        <p className="text-[14px] text-ink-2">{problem.agentLine}</p>
        <div className="flex flex-wrap items-center gap-2">
          <Phrase className="py-1.5 text-[13.5px]">{SAVE_PHRASE}</Phrase>
          <CopyButton
            text={SAVE_PHRASE}
            what="the phrase"
            variant="ghost"
            size="sm"
            className="h-[30px] px-2 font-semibold text-[13.5px] text-primary"
          />
        </div>
      </div>
      <SheetFooter>
        <Button variant="secondary" onClick={onClose} className="font-semibold">
          Close
        </Button>
        {problem.retry ? (
          <Button onClick={onRetry} className="font-semibold">
            Try again
          </Button>
        ) : (
          <NavLink
            to="/skills"
            onClick={onClose}
            className="inline-flex h-10 items-center gap-2 rounded-md bg-accent-tint px-4 font-semibold text-[14px] text-accent-ink"
          >
            <Bookmark aria-hidden className="size-4" />
            See your skills
          </NavLink>
        )}
      </SheetFooter>
    </div>
  )
}
