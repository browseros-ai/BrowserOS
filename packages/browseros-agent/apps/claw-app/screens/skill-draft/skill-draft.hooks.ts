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
import { useState } from 'react'
import { useNavigate } from 'react-router'
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
  toSkillCreate,
} from './skill-draft.helpers'

/** How often a draft being written is checked, from any view of the task. */
const WRITING_POLL_MS = 2000

const WRITING: SkillDraftStatus = { status: 'writing' }

export function useMakeSkill({
  sessionId,
  skillName,
  enabled,
}: {
  sessionId: string
  /** The skill already saved from this task, from the task summary. */
  skillName?: string
  /** Whether this plan and task can make a skill; nothing is fetched otherwise. */
  enabled: boolean
}) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [sheetOpen, setSheetOpen] = useState(false)
  const variables = { sessionId }
  const statusKey = useSkillDraftStatus.getKey(variables)
  const statusQuery = useSkillDraftStatus({
    variables: enabled && !skillName ? variables : skipToken,
    refetchInterval: (query) =>
      query.state.data?.status === 'writing' ? WRITING_POLL_MS : false,
  })
  const draft = useDraftSkill({
    onSuccess: (result) => queryClient.setQueryData(statusKey, result),
    // The request itself failed (the plan changed, or the server is gone): show the retry view.
    onError: () =>
      queryClient.setQueryData<SkillDraftStatus>(statusKey, {
        status: 'failed',
        failure: { code: 'upstream_error' },
      }),
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
    draft.mutate(variables)
  }

  const start = () => {
    if (isBusy(state)) return
    if (state === 'saved' && status?.skillName) {
      navigate(`/skills/${encodeURIComponent(status.skillName)}`)
      return
    }
    setSheetOpen(true)
    if (state === 'idle' || state === 'error') requestDraft()
  }

  const undo = (name: string) => {
    remove.mutate(
      { name },
      {
        onSuccess: refreshTask,
        onError: (error) => toastApiError('Couldn’t undo that', error),
      },
    )
  }

  const save = (edits: DraftEdits) => {
    const ready = status?.draft
    if (!ready || create.isPending) return
    create.mutate(toSkillCreate(ready, edits, sessionId), {
      onSuccess: (skill) => {
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
        navigate('/skills', { state: { triedSkill: skill.name } })
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
    close: () => setSheetOpen(false),
  }
}

export type MakeSkill = ReturnType<typeof useMakeSkill>
