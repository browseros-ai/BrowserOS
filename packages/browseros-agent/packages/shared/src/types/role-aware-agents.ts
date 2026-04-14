export type BrowserOSAgentRoleId = 'chief-of-staff'

export interface BrowserOSRoleBoundary {
  key: string
  label: string
  description: string
  defaultMode: 'allow' | 'ask' | 'block'
}

export interface BrowserOSRoleTemplate {
  id: BrowserOSAgentRoleId
  name: string
  shortDescription: string
  longDescription: string
  recommendedApps: string[]
  defaultAgentName: string
  bootstrap: {
    agentsMd: string
    soulMd: string
    toolsMd: string
  }
  boundaries: BrowserOSRoleBoundary[]
}

export interface BrowserOSCustomRoleInput {
  name: string
  shortDescription: string
  longDescription: string
  recommendedApps: string[]
  boundaries: BrowserOSRoleBoundary[]
  bootstrap?: {
    agentsMd?: string
    soulMd?: string
    toolsMd?: string
  }
}

export interface RoleAwareCreateAgentInput {
  name: string
  roleId?: BrowserOSAgentRoleId
  customRole?: BrowserOSCustomRoleInput
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
}

export interface BrowserOSAgentRoleSummary {
  roleSource: 'builtin' | 'custom'
  roleId?: BrowserOSAgentRoleId
  roleName: string
  shortDescription: string
}
