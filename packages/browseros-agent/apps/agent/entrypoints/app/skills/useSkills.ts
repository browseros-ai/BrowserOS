import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { useEffect } from 'react'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export type SkillMeta = {
  id: string
  name: string
  description: string
  location: string
  enabled: boolean
  builtIn: boolean
}

export type SkillDetail = SkillMeta & {
  content: string
}

type CreateSkillInput = {
  name: string
  description: string
  content: string
}

type UpdateSkillInput = Partial<CreateSkillInput> & {
  enabled?: boolean
}

const SKILLS_QUERY_KEY = 'skills'
const skillsCache = new Map<string, SkillMeta[]>()

async function fetchSkills(baseUrl: string): Promise<SkillMeta[]> {
  const res = await fetch(`${baseUrl}/skills`)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  const data = await res.json()
  return data.skills
}

async function fetchSkill(baseUrl: string, id: string): Promise<SkillDetail> {
  const res = await fetch(`${baseUrl}/skills/${id}`)
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
  const data = await res.json()
  return data.skill
}

async function postSkill(
  baseUrl: string,
  input: CreateSkillInput,
): Promise<SkillMeta> {
  const res = await fetch(`${baseUrl}/skills`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(input),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new Error(data.error || `HTTP ${res.status}`)
  }
  const data = await res.json()
  return data.skill
}

async function putSkill(
  baseUrl: string,
  id: string,
  input: UpdateSkillInput,
): Promise<SkillMeta> {
  const res = await fetch(`${baseUrl}/skills/${id}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(input),
  })
  if (!res.ok) {
    const data = await res.json().catch(() => ({}))
    throw new Error(data.error || `HTTP ${res.status}`)
  }
  const data = await res.json()
  return data.skill
}

async function removeSkill(baseUrl: string, id: string): Promise<void> {
  const res = await fetch(`${baseUrl}/skills/${id}`, { method: 'DELETE' })
  if (!res.ok) throw new Error(`HTTP ${res.status}`)
}

export function useSkills() {
  const { baseUrl, isLoading: urlLoading } = useAgentServerUrl()
  const queryClient = useQueryClient()
  const cacheKey = baseUrl ?? '__pending__'
  const cachedSkills = skillsCache.get(cacheKey)

  const { data, isLoading, error, refetch } = useQuery<SkillMeta[], Error>({
    queryKey: [SKILLS_QUERY_KEY, baseUrl],
    queryFn: () => fetchSkills(baseUrl as string),
    enabled: !!baseUrl && !urlLoading,
    placeholderData: () => cachedSkills,
  })

  useEffect(() => {
    if (!baseUrl || data === undefined) return
    skillsCache.set(baseUrl, data)
  }, [baseUrl, data])

  const invalidate = () =>
    queryClient.invalidateQueries({ queryKey: [SKILLS_QUERY_KEY] })

  const createMutation = useMutation({
    mutationFn: (input: CreateSkillInput) =>
      postSkill(baseUrl as string, input),
    onSuccess: invalidate,
  })

  const updateMutation = useMutation({
    mutationFn: ({ id, input }: { id: string; input: UpdateSkillInput }) =>
      putSkill(baseUrl as string, id, input),
    onSuccess: invalidate,
  })

  const deleteMutation = useMutation({
    mutationFn: (id: string) => removeSkill(baseUrl as string, id),
    onSuccess: invalidate,
  })

  return {
    skills: data ?? cachedSkills ?? [],
    isLoading:
      (isLoading || urlLoading) && (data ?? cachedSkills) === undefined,
    error,
    refetch,
    createSkill: createMutation.mutateAsync,
    updateSkill: (id: string, input: UpdateSkillInput) =>
      updateMutation.mutateAsync({ id, input }),
    deleteSkill: deleteMutation.mutateAsync,
    fetchSkillDetail: (id: string) => fetchSkill(baseUrl as string, id),
  }
}
