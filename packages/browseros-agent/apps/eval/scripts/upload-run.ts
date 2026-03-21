/**
 * Upload eval runs to R2.
 *
 * Two modes:
 *   bun scripts/upload-run.ts results/browseros-agent-weekly/2026-03-21-1730
 *       → uploads that specific run
 *
 *   bun scripts/upload-run.ts results/browseros-agent-weekly
 *       → finds all timestamped subfolders, uploads any not yet in R2
 *
 * Env vars: EVAL_R2_ACCOUNT_ID, EVAL_R2_ACCESS_KEY_ID, EVAL_R2_SECRET_ACCESS_KEY
 *           EVAL_R2_BUCKET (default: browseros-eval)
 *           EVAL_R2_CDN_BASE_URL (default: https://eval.browseros.com)
 */

import { readdir, readFile, stat } from 'node:fs/promises'
import { basename, dirname, extname, join } from 'node:path'
import {
  GetObjectCommand,
  PutObjectCommand,
  S3Client,
} from '@aws-sdk/client-s3'

const CONCURRENCY = 20

const CONTENT_TYPES: Record<string, string> = {
  '.json': 'application/json',
  '.jsonl': 'application/x-ndjson',
  '.png': 'image/png',
}

interface R2Config {
  accountId: string
  accessKeyId: string
  secretAccessKey: string
  bucket: string
  cdnBaseUrl: string
}

function loadConfig(): R2Config {
  const accountId = process.env.EVAL_R2_ACCOUNT_ID
  const accessKeyId = process.env.EVAL_R2_ACCESS_KEY_ID
  const secretAccessKey = process.env.EVAL_R2_SECRET_ACCESS_KEY

  if (!accountId || !accessKeyId || !secretAccessKey) {
    console.error(
      'Missing required env vars: EVAL_R2_ACCOUNT_ID, EVAL_R2_ACCESS_KEY_ID, EVAL_R2_SECRET_ACCESS_KEY',
    )
    process.exit(1)
  }

  return {
    accountId,
    accessKeyId,
    secretAccessKey,
    bucket: process.env.EVAL_R2_BUCKET || 'browseros-eval',
    cdnBaseUrl: (
      process.env.EVAL_R2_CDN_BASE_URL || 'https://eval.browseros.com'
    ).replace(/\/+$/, ''),
  }
}

function createClient(config: R2Config): S3Client {
  return new S3Client({
    region: 'auto',
    endpoint: `https://${config.accountId}.r2.cloudflarestorage.com`,
    credentials: {
      accessKeyId: config.accessKeyId,
      secretAccessKey: config.secretAccessKey,
    },
  })
}

async function upload(
  client: S3Client,
  bucket: string,
  key: string,
  body: Buffer,
  contentType: string,
) {
  await client.send(
    new PutObjectCommand({
      Bucket: bucket,
      Key: key,
      Body: body,
      ContentType: contentType,
    }),
  )
}

async function collectFiles(dir: string): Promise<string[]> {
  const files: string[] = []
  const entries = await readdir(dir, { withFileTypes: true })
  for (const entry of entries) {
    const full = join(dir, entry.name)
    if (entry.isDirectory()) {
      files.push(...(await collectFiles(full)))
    } else {
      files.push(full)
    }
  }
  return files
}

async function runPool<T>(
  items: T[],
  concurrency: number,
  fn: (item: T) => Promise<void>,
) {
  let i = 0
  const workers = Array.from({ length: concurrency }, async () => {
    while (i < items.length) {
      const idx = i++
      await fn(items[idx])
    }
  })
  await Promise.all(workers)
}

// Check if a run has already been uploaded to R2
async function isUploaded(
  client: S3Client,
  bucket: string,
  runId: string,
): Promise<boolean> {
  try {
    await client.send(
      new GetObjectCommand({
        Bucket: bucket,
        Key: `runs/${runId}/manifest.json`,
      }),
    )
    return true
  } catch {
    return false
  }
}

// Detect if a directory is a run dir (has task subdirs with metadata.json)
// vs a config dir (has timestamped subdirs like 2026-03-21-1730/)
async function isRunDir(dir: string): Promise<boolean> {
  const entries = await readdir(dir, { withFileTypes: true })
  const subdirs = entries.filter((e) => e.isDirectory())
  for (const subdir of subdirs) {
    const metaPath = join(dir, subdir.name, 'metadata.json')
    const metaStat = await stat(metaPath).catch(() => null)
    if (metaStat?.isFile()) return true
  }
  return false
}

async function uploadSingleRun(
  runDir: string,
  runId: string,
  r2Config: R2Config,
  client: S3Client,
): Promise<void> {
  const taskDirs = await readdir(runDir, { withFileTypes: true })
  const taskEntries = taskDirs.filter((d) => d.isDirectory())

  if (taskEntries.length === 0) {
    console.warn(`  No task subdirectories in ${runId}, skipping`)
    return
  }

  const manifestTasks: Record<string, unknown>[] = []
  const jobs: { key: string; filePath: string; contentType: string }[] = []

  // Extract agent config from first task
  let agentConfig: Record<string, unknown> | undefined
  let dataset: string | undefined

  for (const taskDir of taskEntries) {
    const taskId = taskDir.name
    const taskPath = join(runDir, taskId)
    const metaPath = join(taskPath, 'metadata.json')

    let meta: Record<string, unknown> = {}
    try {
      meta = JSON.parse(await readFile(metaPath, 'utf-8'))
    } catch {
      continue
    }

    if (!agentConfig && meta.agent_config)
      agentConfig = meta.agent_config as Record<string, unknown>
    if (!dataset && meta.dataset) dataset = meta.dataset as string

    const files = await collectFiles(taskPath)
    let screenshotCount = 0

    for (const file of files) {
      const relative = file.slice(taskPath.length + 1)
      const ext = extname(file)
      if (relative.startsWith('screenshots/') && ext === '.png')
        screenshotCount++

      jobs.push({
        key: `runs/${runId}/${taskId}/${relative}`,
        filePath: file,
        contentType: CONTENT_TYPES[ext] || 'application/octet-stream',
      })
    }

    manifestTasks.push({
      queryId: meta.query_id || taskId,
      query: meta.query || '',
      startUrl: meta.start_url || '',
      status:
        meta.termination_reason === 'completed'
          ? 'completed'
          : meta.termination_reason || 'unknown',
      durationMs: meta.total_duration_ms || 0,
      screenshotCount: (meta.screenshot_count as number) || screenshotCount,
      graderResults: meta.grader_results || {},
    })
  }

  if (manifestTasks.length === 0) {
    console.warn(`  No completed tasks in ${runId}, skipping`)
    return
  }

  console.log(
    `  Uploading ${jobs.length} files across ${manifestTasks.length} tasks...`,
  )

  let uploaded = 0
  await runPool(jobs, CONCURRENCY, async (job) => {
    const body = await readFile(job.filePath)
    await upload(client, r2Config.bucket, job.key, body, job.contentType)
    uploaded++
    if (uploaded % 50 === 0 || uploaded === jobs.length) {
      console.log(`    ${uploaded}/${jobs.length}`)
    }
  })

  // Read summary.json if it exists
  let summaryData: Record<string, unknown> | undefined
  try {
    summaryData = JSON.parse(
      await readFile(join(runDir, 'summary.json'), 'utf-8'),
    )
  } catch {}

  // Upload manifest
  const manifest = {
    runId,
    uploadedAt: new Date().toISOString(),
    agentConfig,
    dataset,
    summary: summaryData
      ? {
          passRate: summaryData.passRate,
          avgDurationMs: summaryData.avgDurationMs,
        }
      : undefined,
    tasks: manifestTasks,
  }
  const manifestBody = Buffer.from(JSON.stringify(manifest, null, 2))
  await upload(
    client,
    r2Config.bucket,
    `runs/${runId}/manifest.json`,
    manifestBody,
    'application/json',
  )

  // Upload viewer.html to bucket root
  const viewerPath = join(
    import.meta.dir,
    '..',
    'src',
    'dashboard',
    'viewer.html',
  )
  const viewerBody = await readFile(viewerPath)
  await upload(client, r2Config.bucket, 'viewer.html', viewerBody, 'text/html')

  console.log(`  Uploaded ${uploaded + 2} files`)
  console.log(`  ${r2Config.cdnBaseUrl}/viewer.html?run=${runId}`)
}

async function main() {
  const inputDir = process.argv[2]
  if (!inputDir) {
    console.error(
      'Usage:\n' +
        '  bun scripts/upload-run.ts results/config-name/2026-03-21-1730  (specific run)\n' +
        '  bun scripts/upload-run.ts results/config-name                   (all un-uploaded runs)',
    )
    process.exit(1)
  }

  const dirStat = await stat(inputDir).catch(() => null)
  if (!dirStat?.isDirectory()) {
    console.error(`Not a directory: ${inputDir}`)
    process.exit(1)
  }

  const r2Config = loadConfig()
  const client = createClient(r2Config)

  if (await isRunDir(inputDir)) {
    // Single run: results/config-name/2026-03-21-1730
    const timestamp = basename(inputDir)
    const configName = basename(dirname(inputDir))
    const runId = `${configName}-${timestamp}`
    console.log(`Uploading run: ${runId}`)
    await uploadSingleRun(inputDir, runId, r2Config, client)
  } else {
    // Config dir: results/config-name/ — upload all un-uploaded runs
    const configName = basename(inputDir)
    const entries = await readdir(inputDir, { withFileTypes: true })
    const runDirs = entries
      .filter((e) => e.isDirectory())
      .map((e) => e.name)
      .sort()

    if (runDirs.length === 0) {
      console.error('No run subdirectories found')
      process.exit(1)
    }

    console.log(
      `Found ${runDirs.length} runs for config "${configName}", checking R2...`,
    )

    let uploadedCount = 0
    for (const dir of runDirs) {
      const runId = `${configName}-${dir}`
      const alreadyUploaded = await isUploaded(client, r2Config.bucket, runId)
      if (alreadyUploaded) {
        console.log(`  ${runId}: already uploaded, skipping`)
        continue
      }

      console.log(`  ${runId}: uploading...`)
      await uploadSingleRun(join(inputDir, dir), runId, r2Config, client)
      uploadedCount++
    }

    console.log(
      `\nDone. Uploaded ${uploadedCount} new run(s), ${runDirs.length - uploadedCount} already in R2.`,
    )
  }
}

main()
