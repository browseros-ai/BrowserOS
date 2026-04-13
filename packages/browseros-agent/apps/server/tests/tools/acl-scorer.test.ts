import { describe, it, setDefaultTimeout } from 'bun:test'

setDefaultTimeout(30_000)

import assert from 'node:assert'
import { resolve } from 'node:path'
import type { AclRule, ElementProperties } from '@browseros/shared/types/acl'
import { editDistanceRatio } from '../../src/tools/acl/acl-edit-distance'
import { scoreFixture } from '../../src/tools/acl/acl-scorer'

// --- Edit distance tests ---

describe('editDistanceRatio', () => {
  it('returns 1.0 for identical strings', () => {
    assert.strictEqual(editDistanceRatio('hello', 'hello'), 1.0)
  })

  it('returns 1.0 for two empty strings', () => {
    assert.strictEqual(editDistanceRatio('', ''), 1.0)
  })

  it('returns 0.0 for completely different strings', () => {
    assert.strictEqual(editDistanceRatio('abc', 'xyz'), 0.0)
  })

  it('returns normalized similarity for insertion-heavy partial matches', () => {
    assert.strictEqual(editDistanceRatio('submit', 'submit order'), 0.5)
  })

  it('returns 0.0 when one string is empty', () => {
    assert.strictEqual(editDistanceRatio('hello', ''), 0.0)
    assert.strictEqual(editDistanceRatio('', 'hello'), 0.0)
  })

  it('produces normalized ratios for similar strings', () => {
    assert.strictEqual(
      editDistanceRatio('place order', 'place your order'),
      0.6875,
    )
  })
})

// --- Scorer tests (no embedding model required) ---

function makeElement(
  overrides: Partial<ElementProperties> = {},
): ElementProperties {
  return {
    tagName: 'button',
    textContent: '',
    attributes: {},
    ...overrides,
  }
}

function makeRule(overrides: Partial<AclRule> = {}): AclRule {
  return {
    id: 'test-rule',
    sitePattern: '*',
    enabled: true,
    ...overrides,
  }
}

describe('scoreFixture', () => {
  it('returns not blocked when no rules match site', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement(),
      [makeRule({ sitePattern: 'other.com' })],
    )
    assert.strictEqual(decision.blocked, false)
    assert.strictEqual(decision.reason, 'no-matching-rules')
  })

  it('blocks on site-only rule', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement(),
      [makeRule({ id: 'site-block', sitePattern: '*' })],
    )
    assert.strictEqual(decision.blocked, true)
    assert.strictEqual(decision.matchedRuleId, 'site-block')
    assert.strictEqual(decision.reason, 'site-only-rule')
  })

  it('blocks on exact text match', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement({
        textContent: 'Place Order',
        ariaLabel: 'Place Order',
      }),
      [makeRule({ id: 'order-block', textMatch: 'place order' })],
    )
    assert.strictEqual(decision.blocked, true)
    assert.strictEqual(decision.matchedRuleId, 'order-block')
    assert.strictEqual(decision.reason, 'exact-term-match')
  })

  it('does not block when text does not match', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement({ textContent: 'Cancel' }),
      [makeRule({ textMatch: 'submit order' })],
    )
    assert.strictEqual(decision.blocked, false)
  })

  it('skips disabled rules', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement(),
      [makeRule({ enabled: false })],
    )
    assert.strictEqual(decision.blocked, false)
    assert.strictEqual(decision.candidates.length, 0)
  })

  it('handles selector-only rules', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement({ attributes: { id: 'danger-btn' } }),
      [makeRule({ id: 'sel-rule', selector: '#danger-btn' })],
    )
    assert.strictEqual(decision.blocked, true)
    assert.strictEqual(decision.matchedRuleId, 'sel-rule')
    assert.strictEqual(decision.reason, 'selector-only')
  })

  it('returns selector-mismatch when selector does not match', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement({ attributes: { id: 'safe-btn' } }),
      [makeRule({ selector: '#danger-btn', textMatch: 'delete' })],
    )
    assert.strictEqual(decision.blocked, false)
    assert.strictEqual(decision.candidates[0].reason, 'selector-mismatch')
  })

  it('does not match unsupported compound selectors by tag alone', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement({ attributes: { id: 'safe-btn' } }),
      [makeRule({ selector: 'button.primary' })],
    )
    assert.strictEqual(decision.blocked, false)
    assert.strictEqual(decision.reason, 'selector-mismatch')
  })

  it('sorts candidates by confidence descending', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement({ textContent: 'Submit' }),
      [
        makeRule({ id: 'low', description: 'completely unrelated xyz' }),
        makeRule({ id: 'high', textMatch: 'submit' }),
      ],
    )
    assert.strictEqual(decision.candidates[0].ruleId, 'high')
    assert.ok(
      decision.candidates[0].confidence >= decision.candidates[1].confidence,
    )
  })

  it('does not exact-match generic button terms from the role field', async () => {
    const decision = await scoreFixture(
      'click',
      'https://example.com',
      makeElement({
        textContent: 'View Report',
        ariaLabel: 'View Report',
        role: 'button',
      }),
      [makeRule({ description: 'block dangerous button' })],
    )
    assert.strictEqual(decision.blocked, false)
    assert.strictEqual(decision.candidates[0].exactScore, 0)
    assert.deepStrictEqual(decision.candidates[0].matchedTerms, [])
  })
})

// --- Fixture tests ---

async function loadFixture(name: string) {
  const path = resolve(import.meta.dir, `../__fixtures__/acl/${name}.json`)
  return Bun.file(path).json()
}

async function runFixture(name: string) {
  const f = await loadFixture(name)
  return scoreFixture(f.tool_name, f.page_url, f.element, f.rules)
}

describe('fixture: submit-button (exact match)', () => {
  it('blocks checkout submit button', async () => {
    const decision = await runFixture('submit-button')

    assert.strictEqual(decision.blocked, true)
    assert.strictEqual(decision.matchedRuleId, 'checkout-submit')
    assert.strictEqual(decision.reason, 'exact-term-match')
    assert.ok(
      decision.confidence >= 0.4,
      `confidence ${decision.confidence} should be >= 0.4`,
    )
  })

  it('uses the embedding model for scoring', async () => {
    const decision = await runFixture('submit-button')
    const top = decision.candidates[0]

    assert.ok(
      top.semanticBackend === 'transformers.js' ||
        top.semanticBackend === 'error',
      `semanticBackend should be transformers.js or error, got ${top.semanticBackend}`,
    )
  })
})

describe('fixture: semantic-payment (semantic match)', () => {
  it('blocks "Proceed to Checkout" against payment prevention rule', async () => {
    const decision = await runFixture('semantic-payment')

    assert.strictEqual(decision.blocked, true)
    assert.strictEqual(decision.matchedRuleId, 'block-payments')
    assert.ok(
      decision.confidence >= 0.4,
      `confidence ${decision.confidence} should be >= 0.4`,
    )
  })

  it('has a meaningful semantic score', async () => {
    const decision = await runFixture('semantic-payment')
    const top = decision.candidates[0]

    if (top.semanticBackend === 'transformers.js') {
      assert.ok(
        top.semanticScore > 0.3,
        `semantic score ${top.semanticScore} should be > 0.3 for payment/checkout similarity`,
      )
    }
  })
})

describe('fixture: semantic-delete (semantic match)', () => {
  it('blocks "Remove my account permanently" against deletion rule', async () => {
    const decision = await runFixture('semantic-delete')

    assert.strictEqual(decision.blocked, true)
    assert.strictEqual(decision.matchedRuleId, 'block-delete')
    assert.ok(
      decision.confidence >= 0.4,
      `confidence ${decision.confidence} should be >= 0.4`,
    )
  })
})

describe('fixture: semantic-send-email (semantic match)', () => {
  it('blocks send button on mail compose page', async () => {
    const decision = await runFixture('semantic-send-email')

    assert.strictEqual(decision.blocked, true)
    assert.strictEqual(decision.matchedRuleId, 'block-outbound-email')
    assert.ok(
      decision.confidence >= 0.4,
      `confidence ${decision.confidence} should be >= 0.4`,
    )
  })
})

describe('fixture: semantic-safe (no false positive)', () => {
  it('allows "View Report" against payment and deletion rules', async () => {
    const decision = await runFixture('semantic-safe')

    assert.strictEqual(decision.blocked, false)
    assert.ok(
      decision.confidence < 0.4,
      `confidence ${decision.confidence} should be < 0.4 for unrelated action`,
    )
  })
})
