/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import {
  disableInAppBrowserPlugin,
  IN_APP_BROWSER_PLUGIN_KEY,
  verifyBrowserDisabled,
} from '../../../../src/lib/agents/host-acp/codex-home'

function browserEnabled(toml: string): unknown {
  const parsed = Bun.TOML.parse(toml) as {
    plugins?: Record<string, { enabled?: unknown }>
  }
  return parsed.plugins?.[IN_APP_BROWSER_PLUGIN_KEY]?.enabled
}

describe('disableInAppBrowserPlugin', () => {
  it('flips an existing enabled = true to false', () => {
    const src = `model = "gpt-5.5"\n\n[plugins."browser@openai-bundled"]\nenabled = true\n`
    const out = disableInAppBrowserPlugin(src)
    expect(browserEnabled(out)).toBe(false)
    expect(out).toContain('model = "gpt-5.5"')
  })

  it('is idempotent when already disabled', () => {
    const src = `[plugins."browser@openai-bundled"]\nenabled = false\n`
    expect(disableInAppBrowserPlugin(src)).toBe(
      disableInAppBrowserPlugin(disableInAppBrowserPlugin(src)),
    )
    expect(browserEnabled(disableInAppBrowserPlugin(src))).toBe(false)
  })

  it('inserts enabled = false when the table has no enabled key', () => {
    const src = `[plugins."browser@openai-bundled"]\nother = 1\n`
    const out = disableInAppBrowserPlugin(src)
    expect(browserEnabled(out)).toBe(false)
    expect(Bun.TOML.parse(out)).toMatchObject({
      plugins: { [IN_APP_BROWSER_PLUGIN_KEY]: { other: 1 } },
    })
  })

  it('appends the table when it is absent', () => {
    const src = `model = "gpt-5.5"\n`
    const out = disableInAppBrowserPlugin(src)
    expect(browserEnabled(out)).toBe(false)
  })

  it('appends the table for empty input', () => {
    expect(browserEnabled(disableInAppBrowserPlugin(''))).toBe(false)
  })

  it('does not leak into a following table', () => {
    const src = `[plugins."browser@openai-bundled"]\n\n[other]\nenabled = true\n`
    const out = disableInAppBrowserPlugin(src)
    const parsed = Bun.TOML.parse(out) as {
      plugins: Record<string, { enabled: unknown }>
      other: { enabled: unknown }
    }
    expect(parsed.plugins[IN_APP_BROWSER_PLUGIN_KEY].enabled).toBe(false)
    expect(parsed.other.enabled).toBe(true)
  })

  it('only touches the target table, preserving siblings', () => {
    const src = `[plugins."documents@openai-primary-runtime"]\nenabled = true\n\n[plugins."browser@openai-bundled"]\nenabled = true\n`
    const parsed = Bun.TOML.parse(disableInAppBrowserPlugin(src)) as {
      plugins: Record<string, { enabled: unknown }>
    }
    expect(parsed.plugins['documents@openai-primary-runtime'].enabled).toBe(
      true,
    )
    expect(parsed.plugins[IN_APP_BROWSER_PLUGIN_KEY].enabled).toBe(false)
  })
})

describe('verifyBrowserDisabled', () => {
  it('accepts an edit that only disables the plugin', () => {
    const src = `model = "gpt-5.5"\n\n[plugins."browser@openai-bundled"]\nenabled = true\n`
    expect(verifyBrowserDisabled(src, disableInAppBrowserPlugin(src))).toBe(
      true,
    )
  })

  it('rejects an edit that changes an unrelated field', () => {
    const src = `model = "gpt-5.5"\n\n[plugins."browser@openai-bundled"]\nenabled = true\n`
    const tampered = `model = "gpt-5.6"\n\n[plugins."browser@openai-bundled"]\nenabled = false\n`
    expect(verifyBrowserDisabled(src, tampered)).toBe(false)
  })

  it('rejects when the edit did not disable the plugin', () => {
    const src = `[plugins."browser@openai-bundled"]\nenabled = true\n`
    expect(verifyBrowserDisabled(src, src)).toBe(false)
  })

  it('rejects when the edited text is invalid TOML', () => {
    const src = `[plugins."browser@openai-bundled"]\nenabled = true\n`
    expect(verifyBrowserDisabled(src, 'not = = valid toml [')).toBe(false)
  })

  it('rejects when the source is invalid TOML', () => {
    expect(verifyBrowserDisabled('broken = = [', 'anything')).toBe(false)
  })

  it('round-trips a realistic multi-plugin config', () => {
    const src = [
      'model = "gpt-5.5"',
      'notify = ["a", "b"]',
      '',
      '[plugins."documents@openai-primary-runtime"]',
      'enabled = true',
      '',
      '[plugins."browser@openai-bundled"]',
      'enabled = true',
      '',
      '[shell_environment_policy.set]',
      'BROWSER_USE_AVAILABLE_BACKENDS = "chrome,iab"',
      '',
    ].join('\n')
    const out = disableInAppBrowserPlugin(src)
    expect(verifyBrowserDisabled(src, out)).toBe(true)
    expect(browserEnabled(out)).toBe(false)
  })
})
