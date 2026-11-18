/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tasks (skills) surface: the list, one skill's detail, its run history, and
 * create / edit / delete. Mutations invalidate at the call site via
 * `Hook.getKey()`, so the cache surface stays grep-able.
 */

import type {
  Skill,
  SkillCreate,
  SkillDetail,
  SkillFlowSource,
  SkillList,
  SkillRunList,
} from '@browseros/claw-api'
import type {
  SkillNameRequest,
  UpdateSkillRequest,
} from '@browseros/claw-api-client'
import { createMutation, createQuery } from 'react-query-kit'
import { apiClient } from './client'

export const useSkills = createQuery<SkillList>({
  queryKey: ['api', 'skills'],
  fetcher: async () => (await apiClient()).listSkills(),
})

export const useSkill = createQuery<SkillDetail, { name: string }, Error>({
  queryKey: ['api', 'skill'],
  fetcher: async ({ name }) => (await apiClient()).getSkill({ name }),
})

export const useSkillRuns = createQuery<SkillRunList, { name: string }, Error>({
  queryKey: ['api', 'skill-runs'],
  fetcher: async ({ name }) => (await apiClient()).listSkillRuns({ name }),
})

export const useCreateSkill = createMutation<Skill, SkillCreate>({
  mutationFn: async (body) => (await apiClient()).createSkill(body),
})

export const useUpdateSkill = createMutation<SkillDetail, UpdateSkillRequest>({
  mutationFn: async (request) => (await apiClient()).updateSkill(request),
})

export const useDeleteSkill = createMutation<void, SkillNameRequest>({
  mutationFn: async (request) => (await apiClient()).deleteSkill(request),
})

/** The skill's ultrafast flow file, for reading on the skill page. */
export const useSkillFlow = createQuery<
  SkillFlowSource,
  { name: string },
  Error
>({
  queryKey: ['api', 'skill-flow'],
  fetcher: async ({ name }) => (await apiClient()).getSkillFlow({ name }),
})

/** Runs a write skill without the agent: neo does it by itself, including its one click. */
export const useRunWithoutAgent = createMutation<Skill, SkillNameRequest>({
  mutationFn: async (request) => (await apiClient()).approveSkillWrite(request),
})

/** Runs a write skill with the agent again, so it reads each page as it goes. */
export const useRunWithAgent = createMutation<Skill, SkillNameRequest>({
  mutationFn: async (request) => (await apiClient()).revokeSkillWrite(request),
})
