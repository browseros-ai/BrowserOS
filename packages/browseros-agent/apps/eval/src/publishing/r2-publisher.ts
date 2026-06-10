import { readdir, readFile, stat } from 'node:fs/promises'
import { basename, dirname, extname, join } from 'node:path'
import {
  GetObjectCommand,
  PutObjectCommand,
  S3Client,
} from '@aws-sdk/client-s3'
import { readTaskMetrics } from '../reporting/task-metrics'
import type { TaskDatasetMetadata } from '../types'
import {
  buildViewerManifest,
  type ViewerCriterion,
  type ViewerManifestTaskInput,
} from '../viewer/viewer-manifest'
import type {
  R2PublishPathResult,
  R2PublishRunResult,
  R2RunManifest,
  R2UploadConfig,
} from './r2-manifest'

const DEFAULT_CONCURRENCY = 20

const CONTENT_TYPES: Record<string, string> = {
  '.json': 'application/json',
  '.jsonl': 'application/x-ndjson',
  '.png': 'image/png',
  '.html': 'text/html',
}

export interface R2Client {
  send(command: unknown): Promise<unknown>
}

export interface R2PublisherOptions {
  config: R2UploadConfig
  client?: R2Client
  viewerPath?: string
  concurrency?: number
  now?: () => Date
}

interface UploadJob {
  key: string
  filePath: string
  contentType: string
}

interface TaskDirEntry {
  taskId: string
  taskPath: string
}

export function contentTypeForPath(filePath: string): string {
  return CONTENT_TYPES[extname(filePath)] || 'application/octet-stream'
}

function loadR2ConfigFromEnv(
  env: Record<string, string | undefined> = process.env,
): R2UploadConfig {
  const accountId = env.EVAL_R2_ACCOUNT_ID
  const accessKeyId = env.EVAL_R2_ACCESS_KEY_ID
  const secretAccessKey = env.EVAL_R2_SECRET_ACCESS_KEY

  if (!accountId || !accessKeyId || !secretAccessKey) {
    throw new Error(
      'Missing required env vars: EVAL_R2_ACCOUNT_ID, EVAL_R2_ACCESS_KEY_ID, EVAL_R2_SECRET_ACCESS_KEY',
    )
  }

  return {
    accountId,
    accessKeyId,
    secretAccessKey,
    bucket: env.EVAL_R2_BUCKET || 'browseros-eval',
    cdnBaseUrl: (
      env.EVAL_R2_CDN_BASE_URL || 'https://eval.browseros.com'
    ).replace(/\/+$/, ''),
  }
}

function createR2Client(config: R2UploadConfig): S3Client {
  return new S3Client({
    region: 'auto',
    endpoint: `https://${config.accountId}.r2.cloudflarestorage.com`,
    credentials: {
      accessKeyId: config.accessKeyId,
      secretAccessKey: config.secretAccessKey,
    },
  })
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
): Promise<void> {
  let i = 0
  const workers = Array.from({ length: concurrency }, async () => {
    while (i < items.length) {
      const idx = i++
      await fn(items[idx])
    }
  })
  await Promise.all(workers)
}

async function hasMetadata(dir: string): Promise<boolean> {
  const metaStat = await stat(join(dir, 'metadata.json')).catch(() => null)
  return !!metaStat?.isFile()
}

async function findTaskDirs(runDir: string): Promise<TaskDirEntry[]> {
  const entries = await readdir(runDir, { withFileTypes: true })
  const legacyTasks: TaskDirEntry[] = []
  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name === 'tasks') continue
    const taskPath = join(runDir, entry.name)
    if (await hasMetadata(taskPath)) {
      legacyTasks.push({
        taskId: entry.name,
        taskPath,
      })
    }
  }

  const tasksRoot = join(runDir, 'tasks')
  const canonicalEntries = await readdir(tasksRoot, {
    withFileTypes: true,
  }).catch(() => [])
  const canonicalTasks: TaskDirEntry[] = []
  for (const entry of canonicalEntries) {
    if (!entry.isDirectory()) continue
    const taskPath = join(tasksRoot, entry.name)
    if (await hasMetadata(taskPath)) {
      canonicalTasks.push({
        taskId: entry.name,
        taskPath,
      })
    }
  }

  return legacyTasks.length > 0 ? legacyTasks : canonicalTasks
}

async function isRunDir(dir: string): Promise<boolean> {
  return (await findTaskDirs(dir)).length > 0
}

async function collectRunRootFiles(runDir: string): Promise<UploadJob[]> {
  const entries = await readdir(runDir, { withFileTypes: true })
  return entries
    .filter((entry) => entry.isFile())
    .map((entry) => {
      const filePath = join(runDir, entry.name)
      return {
        key: entry.name,
        filePath,
        contentType: contentTypeForPath(filePath),
      }
    })
}

function extractPerCriterion(
  meta: Record<string, unknown>,
): ViewerCriterion[] | undefined {
  const graders = meta.grader_results as
    | Record<string, { details?: { per_criterion?: unknown } }>
    | undefined
  const list = graders?.agisdk_state_diff?.details?.per_criterion
  if (!Array.isArray(list) || list.length === 0) return undefined

  const out: ViewerCriterion[] = []
  for (const entry of list) {
    if (entry && typeof entry === 'object') {
      const e = entry as {
        passed?: unknown
        softened?: unknown
        detail?: unknown
      }
      out.push({
        passed: e.passed === true,
        ...(e.softened === true ? { softened: true } : {}),
        detail: e.detail,
      })
    }
  }
  return out
}

function statusFromMetadata(meta: Record<string, unknown>): string {
  return meta.termination_reason === 'completed'
    ? 'completed'
    : ((meta.termination_reason as string | undefined) ?? 'unknown')
}

function runIdForDir(runDir: string): string {
  const timestamp = basename(runDir)
  const configName = basename(dirname(runDir))
  return `${configName}-${timestamp}`
}

/** Publishes eval artifacts in the viewer-compatible R2 layout. */
export class R2Publisher {
  private readonly client: R2Client
  private readonly config: R2UploadConfig
  private readonly viewerPath: string
  private readonly concurrency: number
  private readonly now: () => Date

  constructor(options: R2PublisherOptions) {
    this.config = options.config
    this.client = options.client ?? createR2Client(options.config)
    this.viewerPath =
      options.viewerPath ??
      join(import.meta.dirname, '..', 'dashboard', 'viewer.html')
    this.concurrency = options.concurrency ?? DEFAULT_CONCURRENCY
    this.now = options.now ?? (() => new Date())
  }

  async isUploaded(runId: string): Promise<boolean> {
    try {
      await this.client.send(
        new GetObjectCommand({
          Bucket: this.config.bucket,
          Key: `runs/${runId}/manifest.json`,
        }),
      )
      return true
    } catch {
      return false
    }
  }

  async publishPath(inputDir: string): Promise<R2PublishPathResult> {
    const dirStat = await stat(inputDir).catch(() => null)
    if (!dirStat?.isDirectory()) {
      throw new Error(`Not a directory: ${inputDir}`)
    }

    if (await isRunDir(inputDir)) {
      const result = await this.publishRun(inputDir, runIdForDir(inputDir))
      return { uploadedRuns: [result], skippedRuns: [] }
    }

    const configName = basename(inputDir)
    const entries = await readdir(inputDir, { withFileTypes: true })
    const runDirs = entries
      .filter((entry) => entry.isDirectory())
      .map((entry) => entry.name)
      .sort()

    if (runDirs.length === 0) {
      throw new Error('No run subdirectories found')
    }

    const uploadedRuns: R2PublishRunResult[] = []
    const skippedRuns: string[] = []
    for (const dir of runDirs) {
      const runId = `${configName}-${dir}`
      if (await this.isUploaded(runId)) {
        skippedRuns.push(runId)
        continue
      }
      uploadedRuns.push(await this.publishRun(join(inputDir, dir), runId))
    }

    return { uploadedRuns, skippedRuns }
  }

  async publishRun(
    runDir: string,
    runId: string = runIdForDir(runDir),
  ): Promise<R2PublishRunResult> {
    const taskEntries = await findTaskDirs(runDir)

    if (taskEntries.length === 0) {
      throw new Error(`No task subdirectories in ${runId}`)
    }

    const manifestTasks: ViewerManifestTaskInput[] = []
    const jobs: UploadJob[] = (await collectRunRootFiles(runDir)).map(
      (job) => ({
        ...job,
        key: `runs/${runId}/${job.key}`,
      }),
    )
    let agentConfig: Record<string, unknown> | undefined
    let dataset: string | undefined

    for (const taskDirEntry of taskEntries) {
      const { taskId, taskPath } = taskDirEntry
      const meta = await this.readMetadata(taskPath)
      if (!meta) continue

      if (!agentConfig && meta.agent_config) {
        agentConfig = meta.agent_config as Record<string, unknown>
      }
      if (!dataset && meta.dataset) dataset = meta.dataset as string

      const files = await collectFiles(taskPath)
      let screenshotCount = 0
      for (const file of files) {
        const relative = file.slice(taskPath.length + 1)
        if (relative.startsWith('screenshots/') && extname(file) === '.png') {
          screenshotCount++
        }
        // Keep legacy keys during the manifest v2 rollout so cached viewers and
        // old manifests can still resolve task artifacts.
        jobs.push({
          key: `runs/${runId}/${taskId}/${relative}`,
          filePath: file,
          contentType: contentTypeForPath(file),
        })
        jobs.push({
          key: `runs/${runId}/tasks/${taskId}/${relative}`,
          filePath: file,
          contentType: contentTypeForPath(file),
        })
      }

      const taskMetadata = meta.task_metadata as TaskDatasetMetadata | undefined
      const perCriterion = extractPerCriterion(meta)
      const finalAnswer = meta.final_answer
      manifestTasks.push({
        queryId: (meta.query_id as string | undefined) || taskId,
        artifactId: taskId,
        query: (meta.query as string | undefined) || '',
        startUrl: (meta.start_url as string | undefined) || '',
        status: statusFromMetadata(meta),
        durationMs: (meta.total_duration_ms as number | undefined) || 0,
        screenshotCount:
          (meta.screenshot_count as number | undefined) || screenshotCount,
        graderResults:
          (meta.grader_results as ViewerManifestTaskInput['graderResults']) ||
          {},
        metrics: await readTaskMetrics(taskPath, meta, screenshotCount),
        ...(taskMetadata ? { taskMetadata } : {}),
        ...(perCriterion ? { perCriterion } : {}),
        ...(typeof finalAnswer === 'string' || finalAnswer === null
          ? { finalAnswer: finalAnswer as string | null }
          : {}),
      })
    }

    if (manifestTasks.length === 0) {
      throw new Error(`No completed tasks in ${runId}`)
    }

    let uploaded = 0
    await runPool(jobs, this.concurrency, async (job) => {
      await this.uploadFile(job)
      uploaded++
    })

    const manifest = await this.buildManifest(
      runDir,
      runId,
      agentConfig,
      dataset,
      manifestTasks,
    )
    await this.uploadBuffer(
      `runs/${runId}/manifest.json`,
      Buffer.from(JSON.stringify(manifest, null, 2)),
      'application/json',
    )
    await this.uploadBuffer(
      'viewer.html',
      await readFile(this.viewerPath),
      'text/html',
    )

    return {
      runId,
      uploadedFiles: uploaded + 2,
      viewerUrl: `${this.config.cdnBaseUrl}/viewer.html?run=${encodeURIComponent(runId)}`,
      manifest,
    }
  }

  private async readMetadata(
    taskPath: string,
  ): Promise<Record<string, unknown> | null> {
    try {
      return JSON.parse(
        await readFile(join(taskPath, 'metadata.json'), 'utf-8'),
      ) as Record<string, unknown>
    } catch {
      return null
    }
  }

  private async buildManifest(
    runDir: string,
    runId: string,
    agentConfig: Record<string, unknown> | undefined,
    dataset: string | undefined,
    tasks: ViewerManifestTaskInput[],
  ): Promise<R2RunManifest> {
    let summaryData: Record<string, unknown> | undefined
    try {
      summaryData = JSON.parse(
        await readFile(join(runDir, 'summary.json'), 'utf-8'),
      ) as Record<string, unknown>
    } catch {}
    const reportStat = await stat(join(runDir, 'report.html')).catch(() => null)

    return buildViewerManifest({
      runId,
      uploadedAt: this.now().toISOString(),
      reportPath: reportStat?.isFile() ? 'report.html' : undefined,
      agentConfig,
      dataset,
      summary: summaryData
        ? {
            passRate: summaryData.passRate,
            avgDurationMs: summaryData.avgDurationMs,
          }
        : undefined,
      tasks,
    })
  }

  private async uploadFile(job: UploadJob): Promise<void> {
    await this.uploadBuffer(
      job.key,
      await readFile(job.filePath),
      job.contentType,
    )
  }

  private async uploadBuffer(
    key: string,
    body: Buffer,
    contentType: string,
  ): Promise<void> {
    await this.client.send(
      new PutObjectCommand({
        Bucket: this.config.bucket,
        Key: key,
        Body: body,
        ContentType: contentType,
      }),
    )
  }
}

export async function publishPathToR2(
  inputDir: string,
): Promise<R2PublishPathResult> {
  const config = loadR2ConfigFromEnv()
  return new R2Publisher({ config }).publishPath(inputDir)
}
