/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Unit tests for the in-process `permissions.check` API. Exercises
 * the precedence ladder: site-rule clamp → agent verdict → catalog
 * default → unknown-verb safety default.
 */

import { describe, expect, test } from 'bun:test'
import type { NewAgentValues } from '../../src/routes/agents/schemas'
import * as agentsService from '../../src/routes/agents/service'
import * as siteRulesService from '../../src/routes/site-rules/service'
import * as permissions from '../../src/services/permissions'
import { withTempBrowserosDir } from '../_helpers/temp-browseros-dir'

function makeProfile(overrides: Partial<NewAgentValues> = {}): NewAgentValues {
  return {
    name: 'Cowork . Finance ops',
    harness: 'Claude Cowork',
    loginMode: 'profile',
    selectedSites: [],
    approvals: {
      submit: 'Auto',
      payment: 'Block',
      delete: 'Ask',
      upload: 'Ask',
      navigate: 'Ask',
      input: 'Auto',
    },
    aclRuleIds: [],
    customAclRules: [],
    ...overrides,
  }
}

describe('permissions.check', () => {
  test('site rule clamps an agent that wanted Auto', async () => {
    await withTempBrowserosDir(async () => {
      const agent = await agentsService.create(makeProfile())
      await siteRulesService.add({
        label: 'Wire',
        domain: 'mercury.com',
        action: 'payments',
      })
      const result = await permissions.check({
        agentId: agent.id,
        verb: 'payment',
        domain: 'mercury.com',
      })
      // Agent's payment verdict was Block, so even without the site
      // rule we'd block; flip to submit to prove the rule wins over
      // an Auto verdict.
      expect(result).toEqual({ verdict: 'block', source: 'site-rule' })
    })
  })

  test('site rule overrides an agent Auto verdict for submit', async () => {
    await withTempBrowserosDir(async () => {
      const agent = await agentsService.create(
        makeProfile({
          approvals: {
            submit: 'Auto',
            payment: 'Block',
            delete: 'Ask',
            upload: 'Ask',
            navigate: 'Ask',
            input: 'Auto',
          },
        }),
      )
      await siteRulesService.add({
        label: 'Concur',
        domain: 'concur.com',
        action: 'submit',
      })
      const blocked = await permissions.check({
        agentId: agent.id,
        verb: 'submit',
        domain: 'concur.com',
      })
      expect(blocked).toEqual({ verdict: 'block', source: 'site-rule' })

      // Same agent + same verb on a different domain stays Auto.
      const allowed = await permissions.check({
        agentId: agent.id,
        verb: 'submit',
        domain: 'docs.google.com',
      })
      expect(allowed).toEqual({ verdict: 'auto', source: 'agent' })
    })
  })

  test('site rule with wildcard matches subdomains', async () => {
    await withTempBrowserosDir(async () => {
      const agent = await agentsService.create(makeProfile())
      await siteRulesService.add({
        label: 'Stripe',
        domain: '*.stripe.com',
        action: 'payments',
      })
      const sub = await permissions.check({
        agentId: agent.id,
        verb: 'payment',
        domain: 'api.stripe.com',
      })
      expect(sub.source).toBe('site-rule')
      // Apex stripe.com is NOT clamped by `*.stripe.com` (apex needs a
      // separate rule). Falls through to the agent verdict (Block).
      const apex = await permissions.check({
        agentId: agent.id,
        verb: 'payment',
        domain: 'stripe.com',
      })
      expect(apex).toEqual({ verdict: 'block', source: 'agent' })
    })
  })

  test('agent verdict wins when no site rule matches', async () => {
    await withTempBrowserosDir(async () => {
      const agent = await agentsService.create(makeProfile())
      const result = await permissions.check({
        agentId: agent.id,
        verb: 'submit',
        domain: 'docs.google.com',
      })
      expect(result).toEqual({ verdict: 'auto', source: 'agent' })
    })
  })

  test('catalog default applies when the agent is missing', async () => {
    await withTempBrowserosDir(async () => {
      const result = await permissions.check({
        agentId: 'ghost',
        verb: 'payment',
        domain: 'stripe.com',
      })
      expect(result).toEqual({
        verdict: 'block',
        source: 'permission-default',
      })
    })
  })

  test('catalog default applies when the agent profile omits the verb', async () => {
    await withTempBrowserosDir(async () => {
      const agent = await agentsService.create(
        makeProfile({
          approvals: {
            submit: 'Auto',
            // payment intentionally omitted
            delete: 'Ask',
            upload: 'Ask',
            navigate: 'Ask',
            input: 'Auto',
          },
        }),
      )
      const result = await permissions.check({
        agentId: agent.id,
        verb: 'payment',
        domain: 'stripe.com',
      })
      expect(result).toEqual({
        verdict: 'block',
        source: 'permission-default',
      })
    })
  })

  test('unknown verb returns block from the permission-default source', async () => {
    await withTempBrowserosDir(async () => {
      const agent = await agentsService.create(makeProfile())
      const result = await permissions.check({
        agentId: agent.id,
        verb: 'not-a-real-verb',
        domain: 'example.com',
      })
      expect(result).toEqual({
        verdict: 'block',
        source: 'permission-default',
      })
    })
  })

  test('traversal-shaped agentId resolves to catalog default (defence in depth)', async () => {
    await withTempBrowserosDir(async () => {
      const result = await permissions.check({
        agentId: '../config',
        verb: 'submit',
        domain: 'example.com',
      })
      expect(result.source).toBe('permission-default')
      expect(result.verdict).toBe('ask')
    })
  })

  test('admin verb is enforced by matching site rules', async () => {
    await withTempBrowserosDir(async () => {
      const agent = await agentsService.create(makeProfile())
      await siteRulesService.add({
        label: 'Org billing',
        domain: 'admin.*',
        action: 'admin',
      })
      // Configured admin rule must attribute the block to the rule,
      // not to the unknown-verb safety default. If this regresses,
      // the cockpit will show "blocked by default" instead of
      // "blocked by the rule you configured".
      const result = await permissions.check({
        agentId: agent.id,
        verb: 'admin',
        domain: 'admin.workspace.google.com',
      })
      expect(result).toEqual({ verdict: 'block', source: 'site-rule' })

      // Without a matching rule, admin still defaults to block
      // (catalog has no admin verdict), but sourced from the
      // permission-default safety net.
      const noRule = await permissions.check({
        agentId: agent.id,
        verb: 'admin',
        domain: 'docs.google.com',
      })
      expect(noRule).toEqual({
        verdict: 'block',
        source: 'permission-default',
      })
    })
  })

  test('input verb is not domain-scoped: site rules do not clamp it', async () => {
    await withTempBrowserosDir(async () => {
      const agent = await agentsService.create(makeProfile())
      // A submit rule on the same domain must NOT carry over to the
      // input verb space; input falls through to the agent verdict.
      await siteRulesService.add({
        label: 'Concur submit',
        domain: 'concur.com',
        action: 'submit',
      })
      const result = await permissions.check({
        agentId: agent.id,
        verb: 'input',
        domain: 'concur.com',
      })
      expect(result).toEqual({ verdict: 'auto', source: 'agent' })
    })
  })
})
