import type { AclRule } from '@browseros/shared/types/acl'
import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import type { GlobalAclPolicyService } from '../services/acl/global-acl-policy'

const AclRuleSchema = z.object({
  id: z.string(),
  sitePattern: z.string(),
  selector: z.string().optional(),
  textMatch: z.string().optional(),
  description: z.string().optional(),
  enabled: z.boolean(),
})

const PutAclRulesSchema = z.object({
  aclRules: z.array(AclRuleSchema),
})

interface AclRouteDeps {
  policyService: GlobalAclPolicyService
}

export function createAclRoutes(deps: AclRouteDeps) {
  return new Hono()
    .get('/', async (c) => {
      return c.json({ aclRules: deps.policyService.getRules() })
    })
    .put('/', zValidator('json', PutAclRulesSchema), async (c) => {
      const { aclRules } = c.req.valid('json')
      const savedRules = await deps.policyService.setRules(
        aclRules as AclRule[],
      )
      return c.json({ aclRules: savedRules })
    })
}
