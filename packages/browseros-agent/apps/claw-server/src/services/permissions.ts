/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * In-process permission check used by the future browser executor
 * (Phase 3) and run lifecycle (Phase 4). Returns the verdict for a
 * single (agent, verb, domain) tuple plus the source so callers can
 * tell the user WHY their action was clamped.
 *
 * Precedence (highest wins):
 *   1. site-rule match -> block, source: 'site-rule'
 *      Rules are clamps: a matched rule overrides any agent verdict.
 *      Phase 5 treats every matching rule as a hard block; finer-
 *      grained per-rule verdicts can land later without churning the
 *      call site.
 *   2. agent.approvals[verb] -> source: 'agent'
 *   3. catalog defaultVerdict[verb] -> source: 'permission-default'
 *   4. unknown verb -> block, source: 'permission-default'
 *      Defence in depth: a verb the catalog has never heard of is
 *      treated as restricted, not auto.
 *
 * There is NO HTTP route for this. Callers import and invoke
 * directly; the unit tests are the only consumer until the executor
 * lands.
 */

import {
  APPROVAL_CATEGORIES,
  type ApprovalCategory,
  type ApprovalVerdict,
} from '../lib/approval-catalog'
import * as agentsService from '../routes/agents/service'
import type { SiteRuleAction } from '../routes/site-rules/schemas'
import * as siteRulesService from '../routes/site-rules/service'

export type CheckVerdict = 'auto' | 'ask' | 'block'
export type CheckSource = 'agent' | 'site-rule' | 'permission-default'

export interface CheckInput {
  agentId: string
  verb: string
  domain: string
}

export interface CheckResult {
  verdict: CheckVerdict
  source: CheckSource
}

/**
 * Catalog verbs the user configures per agent map onto the coarser
 * site-rule action space. `input` (click & type) is intentionally
 * not domain-scoped: site rules clamp meaningful actions, not every
 * keystroke.
 *
 * `admin` has no catalog counterpart but is a first-class site-rule
 * action; we still want a verb that callers can use to ask "is this
 * admin operation blocked on this domain?" so a configured admin
 * rule attributes the block to `'site-rule'` instead of falling
 * through to the unknown-verb safety default.
 */
const VERB_TO_RULE_ACTION: Record<string, SiteRuleAction> = {
  submit: 'submit',
  payment: 'payments',
  delete: 'delete',
  upload: 'upload',
  navigate: 'navigate',
  admin: 'admin',
}

const CATALOG_BY_ID: Record<string, ApprovalCategory> = Object.fromEntries(
  APPROVAL_CATEGORIES.map((category) => [category.id, category]),
)

function normalize(verdict: ApprovalVerdict): CheckVerdict {
  return verdict.toLowerCase() as CheckVerdict
}

export async function check(input: CheckInput): Promise<CheckResult> {
  const { agentId, verb, domain } = input

  if (domain) {
    const ruleAction = VERB_TO_RULE_ACTION[verb]
    if (ruleAction) {
      const matches = await siteRulesService.findMatching(domain, ruleAction)
      if (matches.length > 0) {
        return { verdict: 'block', source: 'site-rule' }
      }
    }
  }

  const profile = await agentsService.getDetail(agentId)
  const agentVerdict = profile?.approvals[verb]
  if (agentVerdict) {
    return { verdict: normalize(agentVerdict), source: 'agent' }
  }

  const category = CATALOG_BY_ID[verb]
  if (category) {
    return {
      verdict: normalize(category.defaultVerdict),
      source: 'permission-default',
    }
  }

  return { verdict: 'block', source: 'permission-default' }
}
