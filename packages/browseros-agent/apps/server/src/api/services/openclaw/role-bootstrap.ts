import {
  type BROWSEROS_ROLE_TEMPLATES,
  getBrowserOSRoleTemplate,
} from '@browseros/shared/constants/role-aware-agents'
import type {
  BrowserOSAgentRoleId,
  BrowserOSAgentRoleSummary,
  BrowserOSRoleTemplate,
} from '@browseros/shared/types/role-aware-agents'

type RoleTemplate = (typeof BROWSEROS_ROLE_TEMPLATES)[number]

export interface RoleBootstrapFiles {
  'AGENTS.md': string
  'SOUL.md': string
  'TOOLS.md': string
  '.browseros-role.json': string
}

export function resolveRoleTemplate(
  roleId: BrowserOSAgentRoleId,
): RoleTemplate {
  const role = getBrowserOSRoleTemplate(roleId)
  if (!role) {
    throw new Error(`Unknown BrowserOS role: ${roleId}`)
  }
  return role
}

export function buildRoleBootstrapFiles(input: {
  role: BrowserOSRoleTemplate
  agentName: string
}): RoleBootstrapFiles {
  return {
    'AGENTS.md': input.role.bootstrap.agentsMd,
    'SOUL.md': input.role.bootstrap.soulMd,
    'TOOLS.md': input.role.bootstrap.toolsMd,
    '.browseros-role.json': `${JSON.stringify(
      {
        version: 1,
        roleId: input.role.id,
        roleName: input.role.name,
        createdBy: 'browseros',
        agentName: input.agentName,
      },
      null,
      2,
    )}\n`,
  }
}

export function toRoleSummary(
  role: BrowserOSRoleTemplate,
): BrowserOSAgentRoleSummary {
  return {
    roleId: role.id,
    roleName: role.name,
    shortDescription: role.shortDescription,
  }
}
