import { matchesSitePattern } from '@browseros/shared/acl/match'
import type { AclRule, ElementProperties } from '@browseros/shared/types/acl'
import { logger } from '../../lib/logger'
import { editDistanceRatio } from './acl-edit-distance'
import { computeSemanticSimilarity } from './acl-embeddings'
import { NLTK_STOP_WORDS } from './acl-stopwords'

const EXACT_WEIGHT = 0.25
const FUZZY_WEIGHT = 0.25
const SEMANTIC_WEIGHT = 0.5
const BLOCK_THRESHOLD = 0.4

export interface RuleScore {
  ruleId: string
  blocked: boolean
  confidence: number
  exactScore: number
  fuzzyScore: number
  semanticScore: number
  semanticBackend: string
  selectorMatched: boolean
  siteMatched: boolean
  reason: string
  matchedTerms: string[]
}

export interface MatchDecision {
  blocked: boolean
  toolName: string
  pageUrl: string
  matchedRuleId: string | null
  confidence: number
  reason: string
  candidates: RuleScore[]
}

interface RuleMatchInputs {
  terms: string[]
  ruleText: string
  elementFields: string[]
  elementText: string
}

// --- Text normalization ---

function splitIdentifierWords(value: string): string {
  return value
    .replace(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/([A-Z]+)([A-Z][a-z])/g, '$1 $2')
    .replace(/[_-]+/g, ' ')
}

function normalizeText(value: string): string {
  return splitIdentifierWords(value)
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, ' ')
    .trim()
}

function tokenizeWords(value: string): string[] {
  return normalizeText(value)
    .split(/\s+/)
    .filter((t) => t.length > 0 && /^[a-z0-9]+$/.test(t))
}

function normalizeTerm(term: string): string {
  return tokenizeWords(term).join(' ')
}

function dedupe(values: Iterable<string>): string[] {
  const seen = new Set<string>()
  const result: string[] = []

  for (const v of values) {
    if (v && !seen.has(v)) {
      seen.add(v)
      result.push(v)
    }
  }

  return result
}

function dedupeTextTokens(value: string): string {
  return dedupe(value.split(/\s+/)).join(' ')
}

// --- Selector matching ---

function selectorMatchesProps(
  selector: string,
  props: ElementProperties,
): boolean {
  const tag = props.tagName.toLowerCase()
  const id = props.attributes.id
  const classes = (props.attributes.class ?? '').split(/\s+/).filter(Boolean)

  for (const raw of selector.split(',')) {
    const part = raw.trim()
    if (!part) continue
    if (part.startsWith('#') && id && part === `#${id}`) return true
    if (part.startsWith('.') && classes.some((c) => part === `.${c}`))
      return true
    const match = part.match(/^(\w+)$/)
    if (match && match[1].toLowerCase() === tag) return true
  }
  return false
}

// --- Feature extraction ---

function extractHostTerms(pattern: string): Set<string> {
  const host = pattern.includes('/') ? pattern.split('/')[0] : pattern
  const normalized = tokenizeWords(host.replace(/\*/g, ' '))
  return new Set(normalized.filter((t) => t.length >= 3))
}

function compileRuleTerms(rule: AclRule): string[] {
  const terms: string[] = []

  const textMatch = normalizeTerm(rule.textMatch ?? '')
  if (textMatch) terms.push(textMatch)

  const descriptionRaw = rule.description ?? ''
  const description = normalizeTerm(descriptionRaw)
  if (!description) return dedupe(terms)

  terms.push(description)

  const hostTerms = extractHostTerms(rule.sitePattern)
  const descTokens = tokenizeWords(descriptionRaw)
  const rawTerms = descTokens.filter(
    (t) => t.length >= 3 && !NLTK_STOP_WORDS.has(t) && !hostTerms.has(t),
  )
  terms.push(...rawTerms)

  // Make 2-grams and 3-grams from user-provided rules
  for (const window of [2, 3]) {
    if (rawTerms.length < window) continue
    for (let start = 0; start <= rawTerms.length - window; start++) {
      terms.push(rawTerms.slice(start, start + window).join(' '))
    }
  }

  return dedupe(terms)
}

function buildRuleText(rule: AclRule): string {
  return normalizeText([rule.textMatch ?? '', rule.description ?? ''].join(' '))
}

function buildSearchFields(props: ElementProperties): string[] {
  const attrs = props.attributes ?? {}
  const rawFields = [
    props.labelText ?? '',
    props.ariaLabel ?? '',
    props.textContent,
    attrs.placeholder ?? '',
    attrs.title ?? '',
    attrs.name ?? '',
    attrs.value ?? '',
    attrs.id ?? '',
  ]
  return dedupe(rawFields.filter(Boolean).map(normalizeTerm))
}

function buildSearchText(props: ElementProperties): string {
  return dedupeTextTokens(
    [...buildSearchFields(props), normalizeTerm(props.role ?? '')]
      .filter(Boolean)
      .join(' '),
  )
}

function buildRuleMatchInputs(
  rule: AclRule,
  props: ElementProperties,
): RuleMatchInputs {
  return {
    terms: compileRuleTerms(rule),
    ruleText: buildRuleText(rule),
    elementFields: buildSearchFields(props),
    elementText: buildSearchText(props),
  }
}

// --- Similarity scoring ---

function phraseWindows(text: string, phraseTokenCount: number): string[] {
  const tokens = text.split(/\s+/).filter(Boolean)
  if (tokens.length === 0) return []
  if (phraseTokenCount <= 1) return tokens
  if (tokens.length <= phraseTokenCount) return [tokens.join(' ')]

  const windows: string[] = []
  for (let i = 0; i <= tokens.length - phraseTokenCount; i++) {
    windows.push(tokens.slice(i, i + phraseTokenCount).join(' '))
  }
  return windows
}

function exactScore(terms: string[], fields: string[]): [number, string[]] {
  const matched = terms.filter((term) =>
    fields.some((field) => term && field?.includes(term)),
  )
  return [matched.length > 0 ? 1.0 : 0.0, dedupe(matched)]
}

function fuzzyScore(terms: string[], fields: string[]): number {
  let best = 0

  for (const term of terms) {
    const tokenCount = Math.max(term.split(/\s+/).length, 1)

    for (const field of fields) {
      const candidates = phraseWindows(field, tokenCount)
      if (candidates.length === 0) candidates.push(field)
      for (const candidate of candidates) {
        best = Math.max(best, editDistanceRatio(term, candidate))
      }
    }
  }
  return best
}

function weightedScore(exact: number, fuzzy: number, semantic: number): number {
  return (
    EXACT_WEIGHT * exact + FUZZY_WEIGHT * fuzzy + SEMANTIC_WEIGHT * semantic
  )
}

// --- Rule scoring ---

function hasContentFilter(rule: AclRule): boolean {
  return Boolean(rule.selector || rule.textMatch || rule.description)
}

function scoreSelectorMismatch(rule: AclRule): RuleScore {
  return {
    ruleId: rule.id,
    blocked: false,
    confidence: 0,
    exactScore: 0,
    fuzzyScore: 0,
    semanticScore: 0,
    semanticBackend: 'none',
    selectorMatched: false,
    siteMatched: true,
    reason: 'selector-mismatch',
    matchedTerms: [],
  }
}

function scoreSiteOnlyRule(rule: AclRule, selectorMatched: boolean): RuleScore {
  return {
    ruleId: rule.id,
    blocked: true,
    confidence: 1,
    exactScore: 1,
    fuzzyScore: 1,
    semanticScore: 1,
    semanticBackend: 'site-only',
    selectorMatched,
    siteMatched: true,
    reason: 'site-only-rule',
    matchedTerms: [],
  }
}

function scoreSelectorOnlyRule(
  rule: AclRule,
  selectorMatched: boolean,
): RuleScore {
  const confidence = selectorMatched ? 1 : 0
  return {
    ruleId: rule.id,
    blocked: selectorMatched,
    confidence,
    exactScore: confidence,
    fuzzyScore: confidence,
    semanticScore: confidence,
    semanticBackend: 'selector-only',
    selectorMatched,
    siteMatched: true,
    reason: 'selector-only',
    matchedTerms: [],
  }
}

function determineMatchReason(exact: number, confidence: number): string {
  if (exact >= 1.0) return 'exact-term-match'
  if (confidence >= BLOCK_THRESHOLD) return 'weighted-match'
  return 'below-threshold'
}

async function scoreRule(
  pageUrl: string,
  props: ElementProperties,
  rule: AclRule,
): Promise<RuleScore | null> {
  if (rule.enabled === false) return null
  if (!matchesSitePattern(pageUrl, rule.sitePattern)) return null

  let selectorMatched = true
  if (rule.selector) {
    selectorMatched = selectorMatchesProps(rule.selector, props)
    if (!selectorMatched) return scoreSelectorMismatch(rule)
  }

  if (!hasContentFilter(rule)) return scoreSiteOnlyRule(rule, selectorMatched)

  const inputs = buildRuleMatchInputs(rule, props)
  if (inputs.terms.length === 0)
    return scoreSelectorOnlyRule(rule, selectorMatched)

  const [exact, matchedTerms] = exactScore(inputs.terms, inputs.elementFields)
  const fuzzy = fuzzyScore(inputs.terms, inputs.elementFields)
  const semantic = await computeSemanticSimilarity(
    inputs.ruleText,
    inputs.elementText,
  )
  const confidence =
    Math.round(weightedScore(exact, fuzzy, semantic.score) * 10000) / 10000

  const result: RuleScore = {
    ruleId: rule.id,
    blocked: confidence >= BLOCK_THRESHOLD,
    confidence,
    exactScore: Math.round(exact * 10000) / 10000,
    fuzzyScore: Math.round(fuzzy * 10000) / 10000,
    semanticScore: Math.round(semantic.score * 10000) / 10000,
    semanticBackend: semantic.backend,
    selectorMatched,
    siteMatched: true,
    reason: determineMatchReason(exact, confidence),
    matchedTerms,
  }

  logger.debug('ACL rule scored', {
    ruleId: result.ruleId,
    reason: result.reason,
    confidence: result.confidence,
    exact: result.exactScore,
    fuzzy: result.fuzzyScore,
    semantic: result.semanticScore,
    semanticBackend: result.semanticBackend,
  })

  return result
}

export async function scoreFixture(
  toolName: string,
  pageUrl: string,
  element: ElementProperties,
  rules: AclRule[],
): Promise<MatchDecision> {
  const candidates: RuleScore[] = []

  for (const rule of rules) {
    const score = await scoreRule(pageUrl, element, rule)
    if (score) candidates.push(score)
  }

  candidates.sort((a, b) => b.confidence - a.confidence)

  const top = candidates[0]
  const decision: MatchDecision = {
    blocked: top?.blocked ?? false,
    toolName,
    pageUrl,
    matchedRuleId: top?.blocked ? top.ruleId : null,
    confidence: top?.confidence ?? 0,
    reason: top?.reason ?? 'no-matching-rules',
    candidates,
  }

  if (decision.blocked) {
    logger.info('ACL BLOCKED', {
      toolName,
      pageUrl,
      ruleId: decision.matchedRuleId,
      confidence: decision.confidence,
      reason: decision.reason,
    })
  } else {
    logger.debug('ACL ALLOWED', { toolName, pageUrl, reason: decision.reason })
  }

  return decision
}
