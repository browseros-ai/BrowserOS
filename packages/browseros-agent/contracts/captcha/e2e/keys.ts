import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { homedir, tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

export type KeyMode = 'placeholder' | 'real' | 'none'
const providers = ['CAPSOLVER', 'CAPMONSTER'] as const

export async function configureKeys(mode: KeyMode, source = process.env) {
  const env: Record<string, string | undefined> = {}
  const directory =
    mode === 'placeholder'
      ? await mkdtemp(join(tmpdir(), 'captcha-keys-'))
      : undefined
  const cleanup = async () => {
    if (directory) await rm(directory, { recursive: true, force: true })
  }
  try {
    for (const provider of providers) {
      const name = `BROWSEROS_${provider}_API_KEY_FILE`
      if (directory) {
        env[name] = join(directory, provider.toLowerCase())
        await writeFile(env[name], `captcha-e2e-placeholder-${provider}\n`, {
          mode: 0o600,
        })
      } else if (mode === 'real') {
        const path =
          source[name] ||
          join(
            homedir(),
            '.config/browseros/captcha',
            `${provider.toLowerCase()}-api-key`,
          )
        env[name] = resolve(
          path.startsWith('~/') ? join(homedir(), path.slice(2)) : path,
        )
      } else env[name] = undefined
    }
    return { env, cleanup }
  } catch (error) {
    await cleanup()
    throw error
  }
}
