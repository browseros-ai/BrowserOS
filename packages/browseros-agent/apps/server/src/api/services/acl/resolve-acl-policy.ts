import type { AclRule } from '@browseros/shared/types/acl'
import type { GlobalAclPolicyService } from './global-acl-policy'

export async function resolveAclPolicyForMcpRequest(input: {
  policyService: GlobalAclPolicyService
}): Promise<AclRule[]> {
  return input.policyService.getEnabledRules()
}
