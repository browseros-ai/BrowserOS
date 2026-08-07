import type { R2Config } from '@browseros/build-server-tools'
import { requireEnv, resolveEnv } from '@browseros/shared/env/load'

const UPLOAD_PREFIX = 'cli'

export interface CliUploadConfig {
  r2: R2Config
}

export function loadCliUploadConfig(rootDir: string): CliUploadConfig {
  const resolved = resolveEnv({ rootDir, mode: 'production' })
  const r2 = requireEnv(resolved, [
    'R2_ACCOUNT_ID',
    'R2_ACCESS_KEY_ID',
    'R2_SECRET_ACCESS_KEY',
    'R2_BUCKET',
  ])
  return {
    r2: {
      accountId: r2.R2_ACCOUNT_ID,
      accessKeyId: r2.R2_ACCESS_KEY_ID,
      secretAccessKey: r2.R2_SECRET_ACCESS_KEY,
      bucket: r2.R2_BUCKET,
      downloadPrefix: '',
      uploadPrefix: process.env.R2_UPLOAD_PREFIX ?? UPLOAD_PREFIX,
    },
  }
}
