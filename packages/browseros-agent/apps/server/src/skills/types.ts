// agentskills.io spec — metadata is a string→string map for non-spec fields
export type SkillMetadata = {
  'display-name'?: string
  enabled?: string
  version?: string
  [key: string]: string | undefined
}

// agentskills.io spec — only these fields allowed at top level
export type SkillFrontmatter = {
  name: string
  description: string
  license?: string
  compatibility?: string
  metadata?: SkillMetadata
  'allowed-tools'?: string
}

export type SkillSourceKind = 'builtin' | 'local' | 'external'

export type SkillSourceEntry = {
  id: string
  type: 'external'
  path: string
  enabled: boolean
  label?: string
}

export type SkillsSourcesRegistry = {
  version: 1
  sources: SkillSourceEntry[]
}

export type SkillsRuntimeState = {
  version: 1
  skills: Record<string, { enabled: boolean }>
}

export type SkillConflict = {
  kind: 'duplicate-id'
  collisions: Array<{
    sourceKind: SkillSourceKind
    sourceId: string
    location: string
  }>
}

export type SkillMeta = {
  id: string
  name: string
  description: string
  location: string
  enabled: boolean
  version?: string
  builtIn: boolean
  sourceKind: SkillSourceKind
  sourceId: string
  sourceLabel?: string
  conflict?: SkillConflict
}

export type SkillDetail = SkillMeta & {
  content: string
}

export type CreateSkillInput = {
  name: string
  description: string
  content: string
}

export type UpdateSkillInput = Partial<CreateSkillInput> & {
  enabled?: boolean
}

export type RemoteSkillEntry = {
  id: string
  version: string
  content: string
}

export type RemoteSkillCatalog = {
  version: number
  skills: RemoteSkillEntry[]
}
