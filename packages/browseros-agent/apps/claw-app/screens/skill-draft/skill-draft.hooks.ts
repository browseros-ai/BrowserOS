/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * One task's "Make it a skill" flow: the shared draft status, the draft request, and saving the
 * reviewed draft. Every surface showing the task uses this hook, and they all read the same
 * cached status, so a click on Home shows on the task page too.
 */

import type { SkillDraftStatus } from '@browseros/claw-api'
import { skipToken, useQueryClient } from '@tanstack/react-query'
import { useRef, useState } from 'react'
import { useNavigate } from 'react-router'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { useSessionDetail, useSessions } from '@/modules/api/audit.hooks'
import {
  useCreateSkill,
  useDeleteSkill,
  useDraftSkill,
  useSkillDraftStatus,
  useSkills,
} from '@/modules/api/skills.hooks'
import { toast, toastApiError } from '@/modules/toast/toast'
import {
  type DraftEdits,
  isBusy,
  makeSkillState,
  type SkillCtaSurface,
  savedDraftProps,
  toSkillCreate,
} from './skill-draft.helpers'

/** How often a draft being written is checked, from any view of the task. */
const WRITING_POLL_MS = 2000

const WRITING: SkillDraftStatus = { status: 'writing' }

export function useMakeSkill({
  sessionId,
  skillName,
  enabled,
  surface,
}: {
  sessionId: string
  /** Where the button is, for analytics. */
  surface: SkillCtaSurface
  /** The skill already saved from this task, from the task summary. */
  skillName?: string
  /** Whether this plan and task can make a skill; nothing is fetched otherwise. */
  enabled: boolean
}) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [sheetOpen, setSheetOpen] = useState(false)
  const draftStartedAt = useRef(0)
  const variables = { sessionId }
  const statusKey = useSkillDraftStatus.getKey(variables)
  const statusQuery = useSkillDraftStatus({
    variables: enabled && !skillName ? variables : skipToken,
    refetchInterval: (query) =>
      query.state.data?.status === 'writing' ? WRITING_POLL_MS : false,
  })
  const draft = useDraftSkill({
    onSuccess: (result) => {
      queryClient.setQueryData(statusKey, result)
      if (result.status === 'ready' && result.draft) {
        track(AnalyticsEvent.SkillDraftGenerated, {
          ms: Date.now() - draftStartedAt.current,
          confidence: result.draft.confidence,
          steps: result.draft.steps.length,
          inputs: result.draft.inputs.length,
        })
      } else if (result.status === 'failed') {
        track(AnalyticsEvent.SkillDraftFailed, {
          code: result.failure?.code ?? 'upstream_error',
        })
      }
    },
    // The request itself failed (the plan changed, or the server is gone): show the retry view.
    onError: () => {
      queryClient.setQueryData<SkillDraftStatus>(statusKey, {
        status: 'failed',
        failure: { code: 'upstream_error' },
      })
      track(AnalyticsEvent.SkillDraftFailed, { code: 'upstream_error' })
    },
  })
  const create = useCreateSkill()
  const remove = useDeleteSkill()

  const status: SkillDraftStatus | undefined = skillName
    ? { status: 'saved', skillName }
    : statusQuery.data
  const state = makeSkillState({
    status,
    sheetOpen,
    drafting: draft.isPending,
    saving: create.isPending,
    now: Date.now(),
  })

  const refreshTask = () => {
    void queryClient.invalidateQueries({ queryKey: statusKey })
    void queryClient.invalidateQueries({ queryKey: useSkills.getKey() })
    void queryClient.invalidateQueries({ queryKey: useSessions.getKey() })
    void queryClient.invalidateQueries({
      queryKey: useSessionDetail.getKey(variables),
    })
  }

  /** Writes a draft, unless one is already being written, read or kept for this task. */
  const requestDraft = () => {
    // Read the cache, not this render's state: a second click in the same tick sees the first.
    const cached = queryClient.getQueryData<SkillDraftStatus>(statusKey)
    if (cached?.status === 'writing' || draft.isPending) return
    queryClient.setQueryData(statusKey, WRITING)
    draftStartedAt.current = Date.now()
    draft.mutate(variables)
  }

  const start = () => {
    if (isBusy(state)) return
    if (state === 'saved' && status?.skillName) {
      navigate(`/skills/${encodeURIComponent(status.skillName)}`)
      return
    }
    if (state === 'idle')
      track(AnalyticsEvent.SkillFromTaskClicked, { surface })
    setSheetOpen(true)
    if (state === 'idle' || state === 'error') requestDraft()
  }

  // Saving opens the new skill's page, so this button is gone by the time Undo is clicked and
  // callbacks passed to `mutate` would never run; the promise still settles.
  const undo = (name: string) => {
    remove.mutateAsync({ name }).then(
      () => {
        refreshTask()
        navigate('/skills', { replace: true })
      },
      (error) => toastApiError('Couldn’t undo that', error),
    )
  }

  const save = (edits: DraftEdits) => {
    const ready = status?.draft
    if (!ready || create.isPending) return
    create.mutate(toSkillCreate(ready, edits, sessionId), {
      onSuccess: (skill) => {
        track(AnalyticsEvent.SkillFromTaskSaved, savedDraftProps(ready, edits))
        queryClient.setQueryData<SkillDraftStatus>(statusKey, {
          ...status,
          status: 'saved',
          skillName: skill.name,
        })
        refreshTask()
        setSheetOpen(false)
        toast({
          title: 'Skill saved.',
          status: 'success',
          action: { label: 'Undo', onClick: () => undo(skill.name) },
        })
        navigate(`/skills/${encodeURIComponent(skill.name)}`)
      },
      onError: (error) => toastApiError('Couldn’t save this skill', error),
    })
  }

  return {
    state,
    status,
    sheetOpen,
    start,
    retry: requestDraft,
    save,
    close: () => {
      if (state === 'writing' || state === 'review') {
        track(AnalyticsEvent.SkillDraftDiscarded, { stage: state })
      }
      setSheetOpen(false)
    },
  }
}

export type MakeSkill = ReturnType<typeof useMakeSkill>
