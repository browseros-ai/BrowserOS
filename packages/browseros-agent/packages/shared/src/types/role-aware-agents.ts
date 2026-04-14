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

export interface RoleAwareCreateAgentInput {
  name: string
  roleId?: BrowserOSAgentRoleId
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
}

export interface BrowserOSAgentRoleSummary {
  roleId: BrowserOSAgentRoleId
  roleName: string
  shortDescription: string
}
