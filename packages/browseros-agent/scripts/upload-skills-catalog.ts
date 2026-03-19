import { readdir, readFile, stat } from 'node:fs/promises'
import { join } from 'node:path'
import { PutObjectCommand, S3Client } from '@aws-sdk/client-s3'
import type {
  RemoteSkillCatalog,
  RemoteSkillEntry,
} from '../apps/server/src/skills/types'

const DEFAULTS_DIR = join(import.meta.dir, '../apps/server/src/skills/defaults')
const R2_KEY = 'skills/v1/catalog.json'

function extractVersion(content: string): string {
  const match = content.match(/^\s*version:\s*["']?([^"'\n]+)["']?/m)
  return match?.[1]?.trim() || '1.0'
}

async function generateCatalog(): Promise<RemoteSkillCatalog> {
  const entries = await readdir(DEFAULTS_DIR)
  const skills: RemoteSkillEntry[] = []

  for (const entry of entries) {
    const entryPath = join(DEFAULTS_DIR, entry)
    const info = await stat(entryPath)
    if (!info.isDirectory()) continue

    const skillPath = join(entryPath, 'SKILL.md')
    try {
      const content = await readFile(skillPath, 'utf-8')
      skills.push({ id: entry, version: extractVersion(content), content })
    } catch {
      console.error(`Skipping ${entry}: no SKILL.md found`)
    }
  }

  skills.sort((a, b) => a.id.localeCompare(b.id))
  return { version: 1, skills }
}

function requireEnv(name: string): string {
  const value = process.env[name]
  if (!value) {
    console.error(`Missing required env var: ${name}`)
    process.exit(1)
  }
  return value
}

const accountId = requireEnv('R2_ACCOUNT_ID')
const accessKeyId = requireEnv('R2_ACCESS_KEY_ID')
const secretAccessKey = requireEnv('R2_SECRET_ACCESS_KEY')
const bucket = requireEnv('R2_BUCKET')

const client = new S3Client({
  region: 'auto',
  endpoint: `https://${accountId}.r2.cloudflarestorage.com`,
  credentials: { accessKeyId, secretAccessKey },
})

const catalog = await generateCatalog()
const body = JSON.stringify(catalog, null, 2)

console.log(`Generated catalog with ${catalog.skills.length} skills`)

await client.send(
  new PutObjectCommand({
    Bucket: bucket,
    Key: R2_KEY,
    Body: body,
    ContentType: 'application/json',
    CacheControl: 'public, max-age=300',
  }),
)

console.log(`Uploaded to R2: ${bucket}/${R2_KEY}`)
