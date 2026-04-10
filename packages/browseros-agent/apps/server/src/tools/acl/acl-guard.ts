import { matchesSitePattern } from '@browseros/shared/acl/match'
import type { AclRule } from '@browseros/shared/types/acl'
import type { Browser } from '../../browser/browser'
import { logger } from '../../lib/logger'
import { scoreFixture } from './acl-scorer'

const GUARDED_TOOLS = new Set([
  'click',
  'click_at',
  'fill',
  'type_at',
  'hover',
  'hover_at',
  'drag',
  'drag_at',
  'focus',
  'clear',
  'check',
  'uncheck',
  'select_option',
  'press_key',
  'upload_file',
])

export interface AclCheckResult {
  blocked: boolean
  rule?: AclRule
  pageId?: number
  elementId?: number
}

async function resolveTargetElementId(
  toolName: string,
  args: Record<string, unknown>,
  browser: Browser,
  pageId: number,
): Promise<number | undefined> {
  if (typeof args.element === 'number') return args.element
  if (toolName === 'drag' && typeof args.sourceElement === 'number') {
    return args.sourceElement
  }

  if (typeof args.x === 'number' && typeof args.y === 'number') {
    return (
      (await browser.resolveElementAtPoint(pageId, args.x, args.y)) ?? undefined
    )
  }

  if (
    toolName === 'drag_at' &&
    typeof args.startX === 'number' &&
    typeof args.startY === 'number'
  ) {
    return (
      (await browser.resolveElementAtPoint(pageId, args.startX, args.startY)) ??
      undefined
    )
  }

  return undefined
}

export async function checkAcl(
  toolName: string,
  args: Record<string, unknown>,
  browser: Browser,
  rules: AclRule[],
): Promise<AclCheckResult> {
  if (!GUARDED_TOOLS.has(toolName)) return { blocked: false }
  if (!rules.length) return { blocked: false }

  const pageId = args.page as number | undefined
  if (pageId === undefined) return { blocked: false }

  const pageInfo = await browser.refreshPageInfo(pageId)
  if (!pageInfo) return { blocked: false }

  const siteRules = rules.filter((r) =>
    matchesSitePattern(pageInfo.url, r.sitePattern),
  )
  if (!siteRules.length) return { blocked: false }

  const siteOnlyRule = siteRules.find(
    (r) => !r.selector && !r.textMatch && !r.description,
  )
  if (siteOnlyRule) {
    logger.info('ACL blocked by site-only rule', {
      toolName,
      pageId,
      pageUrl: pageInfo.url,
      ruleId: siteOnlyRule.id,
      sitePattern: siteOnlyRule.sitePattern,
    })
    return { blocked: true, rule: siteOnlyRule, pageId }
  }

  const elementId = await resolveTargetElementId(
    toolName,
    args,
    browser,
    pageId,
  )
  if (elementId === undefined) return { blocked: false }

  const props = await browser.resolveElementProperties(pageId, elementId)
  if (!props) return { blocked: false }

  const decision = await scoreFixture(toolName, pageInfo.url, props, siteRules)

  if (decision.blocked) {
    const matchedRule = decision.matchedRuleId
      ? rules.find((rule) => rule.id === decision.matchedRuleId)
      : undefined
    logger.info('ACL blocked by scorer', {
      toolName,
      pageId,
      pageUrl: pageInfo.url,
      elementId,
      ruleId: decision.matchedRuleId,
      confidence: decision.confidence,
      reason: decision.reason,
    })
    return { blocked: true, rule: matchedRule, pageId, elementId }
  }

  return { blocked: false }
}
