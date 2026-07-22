/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { afterEach, beforeEach, describe, expect, it } from 'bun:test'
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import {
  disableInAppBrowserPlugin,
  IN_APP_BROWSER_PLUGIN_KEY,
  materializeCodexBrowserlessHome,
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

describe('materializeCodexBrowserlessHome', () => {
  let root: string
  let source: string
  let browserosDir: string

  beforeEach(async () => {
    root = await mkdtemp(join(tmpdir(), 'codex-home-test-'))
    source = join(root, 'codex')
    browserosDir = join(root, 'browseros')
    await mkdir(source, { recursive: true })
    await mkdir(browserosDir, { recursive: true })
    await writeFile(join(source, 'auth.json'), '{"token":"x"}')
    await mkdir(join(source, 'plugins'))
    await writeFile(
      join(source, 'config.toml'),
      `model = "gpt-5.5"\n\n[plugins."browser@openai-bundled"]\nenabled = true\n`,
    )
  })

  afterEach(async () => {
    await rm(root, { recursive: true, force: true })
  })

  async function overlayConfig(target: string): Promise<string> {
    return readFile(join(target, 'config.toml'), 'utf8')
  }

  it('disables the plugin and symlinks the rest of the home', async () => {
    const target = await materializeCodexBrowserlessHome({
      browserosDir,
      sourceCodexHome: source,
    })
    expect(target).not.toBeNull()
    const config = await overlayConfig(target as string)
    expect(browserEnabled(config)).toBe(false)
    expect(config).toContain('model = "gpt-5.5"')
    // real state is symlinked, config.toml is a real file
    expect(
      (await lstat(join(target as string, 'auth.json'))).isSymbolicLink(),
    ).toBe(true)
    expect(
      (await lstat(join(target as string, 'config.toml'))).isSymbolicLink(),
    ).toBe(false)
  })

  it('treats a missing config.toml as an empty config (ENOENT)', async () => {
    await rm(join(source, 'config.toml'))
    const target = await materializeCodexBrowserlessHome({
      browserosDir,
      sourceCodexHome: source,
    })
    expect(target).not.toBeNull()
    expect(browserEnabled(await overlayConfig(target as string))).toBe(false)
  })

  it('falls back (null) when config.toml exists but is unreadable', async () => {
    await chmod(join(source, 'config.toml'), 0o000)
    // Skip when the runner can still read it (e.g. running as root).
    const stillReadable = await readFile(join(source, 'config.toml'), 'utf8')
      .then(() => true)
      .catch(() => false)
    await chmod(join(source, 'config.toml'), 0o600)
    if (stillReadable) return
    await chmod(join(source, 'config.toml'), 0o000)
    const target = await materializeCodexBrowserlessHome({
      browserosDir,
      sourceCodexHome: source,
    })
    await chmod(join(source, 'config.toml'), 0o600)
    expect(target).toBeNull()
  })

  it('self-heals: picks up new source entries and prunes removed ones', async () => {
    const first = await materializeCodexBrowserlessHome({
      browserosDir,
      sourceCodexHome: source,
    })
    expect(first).not.toBeNull()
    const target = first as string

    await writeFile(join(source, 'newfile.json'), '{}')
    await rm(join(source, 'auth.json'))
    const second = await materializeCodexBrowserlessHome({
      browserosDir,
      sourceCodexHome: source,
    })
    expect(second).toBe(target)
    expect((await lstat(join(target, 'newfile.json'))).isSymbolicLink()).toBe(
      true,
    )
    await expect(lstat(join(target, 'auth.json'))).rejects.toThrow()
  })

  it('is idempotent across repeated calls', async () => {
    const a = await materializeCodexBrowserlessHome({
      browserosDir,
      sourceCodexHome: source,
    })
    const b = await materializeCodexBrowserlessHome({
      browserosDir,
      sourceCodexHome: source,
    })
    expect(a).toBe(b)
    expect(browserEnabled(await overlayConfig(b as string))).toBe(false)
  })

  it('does not mutate the real config.toml', async () => {
    await materializeCodexBrowserlessHome({
      browserosDir,
      sourceCodexHome: source,
    })
    expect(
      browserEnabled(await readFile(join(source, 'config.toml'), 'utf8')),
    ).toBe(true)
  })
})
