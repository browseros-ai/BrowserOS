import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { homedir, tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

export type KeyMode = 'placeholder' | 'real' | 'none'
const primary = 'BROWSEROS_CAPSOLVER_API_KEY_FILE'

export async function configureKeys(mode: KeyMode, source = process.env) {
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
      const path =
        source[primary] ||
        join(homedir(), '.config/browseros/captcha/capsolver-api-key')
      env[primary] = resolve(
        path.startsWith('~/') ? join(homedir(), path.slice(2)) : path,
      )
    } else env[primary] = undefined
    return { env, cleanup }
  } catch (error) {
    await cleanup()
    throw error
  }
}
