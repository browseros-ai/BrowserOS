import { access, mkdtemp, rm, writeFile } from 'node:fs/promises'
import { homedir, tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

export type KeyMode = 'placeholder' | 'real' | 'none'
const primary = 'BROWSEROS_CAPSOLVER_API_KEY_FILE'
const twocaptcha = 'BROWSEROS_2CAPTCHA_API_KEY_FILE'
export const DEFAULT_KEY_DIR = '.config/browseros/captcha'

/**
 * The key-file variables for a server the runner starts. Never reads a key. `placeholder`: one
 * fake CapSolver file, every inherited key cleared. `real`: CapSolver's and 2Captcha's files from
 * their variables or the default folder; other inherited key paths pass through as they are.
 * `none`: every key cleared.
 */
export async function configureKeys(
  mode: KeyMode,
  source: NodeJS.ProcessEnv = process.env,
  home = homedir(),
) {
  const expand = (path: string) =>
    resolve(path.startsWith('~/') ? join(home, path.slice(2)) : path)
  const env: Record<string, string | undefined> = Object.fromEntries(
    Object.entries(source)
      .filter(([name]) => /^BROWSEROS_.+_API_KEY_FILE$/.test(name))
      .map(([name, value]) => [name, mode === 'real' ? value : undefined]),
  )
  const directory =
    mode === 'placeholder'
      ? await mkdtemp(join(tmpdir(), 'captcha-keys-'))
      : undefined
  const cleanup = async () => {
    if (directory) await rm(directory, { recursive: true, force: true })
  }
  try {
    if (directory) {
      env[primary] = join(directory, 'capsolver')
      await writeFile(env[primary], 'captcha-e2e-placeholder-CAPSOLVER\n', {
        mode: 0o600,
      })
    } else if (mode === 'real') {
      env[primary] = expand(
        source[primary] || join(home, DEFAULT_KEY_DIR, 'capsolver-api-key'),
      )
      const fallback = join(home, DEFAULT_KEY_DIR, '2captcha-api-key')
      if (source[twocaptcha]) env[twocaptcha] = expand(source[twocaptcha])
      else if (
        await access(fallback).then(
          () => true,
          () => false,
        )
      )
        env[twocaptcha] = fallback
    } else env[primary] = undefined
    return { env, cleanup }
  } catch (error) {
    await cleanup()
    throw error
  }
}
