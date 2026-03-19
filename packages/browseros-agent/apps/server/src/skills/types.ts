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

export type SkillMeta = {
  id: string
  name: string
  description: string
  location: string
  enabled: boolean
  version?: string
  builtIn: boolean
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
