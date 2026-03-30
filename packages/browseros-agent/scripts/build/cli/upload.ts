import { existsSync, readdirSync, readFileSync } from 'node:fs'
import { writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { log } from '../log'
import { createR2Client, joinObjectKey, uploadFileToObject } from '../server/r2'
import { type CliUploadConfig, loadCliUploadConfig } from './config'

const CDN_BASE_URL = 'https://cdn.browseros.com'
const JSON_CONTENT_TYPE = 'application/json; charset=utf-8'
const CLI_ARCHIVE_PATTERN =
  /^browseros-cli_(?<version>[^_]+)_(?<os>darwin|linux|windows)_(?<arch>amd64|arm64)\.(?<ext>tar\.gz|zip)$/

const INSTALLERS = [
  {
    filePath: join('apps', 'cli', 'scripts', 'install.sh'),
    objectName: 'install.sh',
    contentType: 'text/x-shellscript; charset=utf-8',
  },
  {
    filePath: join('apps', 'cli', 'scripts', 'install.ps1'),
    objectName: 'install.ps1',
    contentType: 'text/plain; charset=utf-8',
  },
] as const

export interface CliReleaseOptions {
  version: string
  binariesDir: string
}

export interface CliReleaseAsset {
  filename: string
  url: string
  archive_format: 'tar.gz' | 'zip'
  sha256: string
}

export interface CliReleaseManifest {
  version: string
  published_at: string
  tag: string
  assets: Record<string, CliReleaseAsset>
}

interface CliArchiveMetadata {
  filename: string
  version: string
  os: string
  arch: string
  archive_format: 'tar.gz' | 'zip'
}

function resolveRootDir(): string {
  const rootDir = resolve(dirname(fileURLToPath(import.meta.url)), '../../..')
  process.chdir(rootDir)
  return rootDir
}

export async function runCliInstallerUpload(): Promise<void> {
  await uploadCliInstallers(resolveRootDir())
}

export async function runCliRelease(options: CliReleaseOptions): Promise<void> {
  await uploadCliRelease(resolveRootDir(), options)
}

export async function uploadCliInstallers(rootDir: string): Promise<void> {
  const { r2 } = loadCliUploadConfig(rootDir)
  const client = createR2Client(r2)

  log.header('Uploading BrowserOS CLI installer scripts')

  try {
    for (const installer of INSTALLERS) {
      const absolutePath = join(rootDir, installer.filePath)
      if (!existsSync(absolutePath)) {
        throw new Error(`Installer script not found: ${installer.filePath}`)
      }

      const objectKey = joinObjectKey(r2.uploadPrefix, installer.objectName)
      log.step(`Uploading ${installer.filePath}`)
      await uploadFileToObject(client, r2, objectKey, absolutePath, {
        contentType: installer.contentType,
      })
      log.success(`Uploaded ${objectKey}`)
      log.info(`${CDN_BASE_URL}/${objectKey}`)
    }

    log.done('CLI installer upload completed')
  } finally {
    client.destroy()
  }
}

export function parseCliChecksums(contents: string): Map<string, string> {
  const entries = new Map<string, string>()
  for (const rawLine of contents.split('\n')) {
    const line = rawLine.trim()
    if (!line) {
      continue
    }
    const match = line.match(/^([a-f0-9]{64})\s+\*?(.+)$/i)
    if (!match) {
      throw new Error(`Invalid checksum line: ${rawLine}`)
    }
    entries.set(match[2], match[1].toLowerCase())
  }
  return entries
}

export function parseCliArchiveFilename(
  filename: string,
): CliArchiveMetadata | null {
  const match = filename.match(CLI_ARCHIVE_PATTERN)
  if (!match?.groups) {
    return null
  }
  const archive_format = match.groups.ext as 'tar.gz' | 'zip'
  return {
    filename,
    version: match.groups.version,
    os: match.groups.os,
    arch: match.groups.arch,
    archive_format,
  }
}

export function buildCliReleaseManifest(options: {
  version: string
  filenames: string[]
  checksumsContent: string
  published_at?: string
  cdnBaseURL?: string
  uploadPrefix?: string
}): CliReleaseManifest {
  const checksumByFilename = parseCliChecksums(options.checksumsContent)
  const assets: Record<string, CliReleaseAsset> = {}
  const filenames = [...options.filenames].sort()
  const cdnBaseURL = options.cdnBaseURL ?? CDN_BASE_URL
  const uploadPrefix = options.uploadPrefix ?? 'cli'

  for (const filename of filenames) {
    const archive = parseCliArchiveFilename(filename)
    if (archive === null) {
      throw new Error(`Unexpected CLI archive filename: ${filename}`)
    }
    if (archive.version !== options.version) {
      throw new Error(
        `Archive ${filename} does not match release version ${options.version}`,
      )
    }

    const checksum = checksumByFilename.get(filename)
    if (!checksum) {
      throw new Error(`Missing checksum for ${filename}`)
    }

    const assetKey = `${archive.os}/${archive.arch}`
    assets[assetKey] = {
      filename,
      url: `${cdnBaseURL}/${joinObjectKey(uploadPrefix, `v${options.version}`, filename)}`,
      archive_format: archive.archive_format,
      sha256: checksum,
    }
  }

  return {
    version: options.version,
    published_at: options.published_at ?? new Date().toISOString(),
    tag: `browseros-cli-v${options.version}`,
    assets,
  }
}

async function uploadCliManifest(
  client: ReturnType<typeof createR2Client>,
  version: string,
  releaseArchives: string[],
  uploadPrefix: string,
  absoluteBinariesDir: string,
  r2: CliUploadConfig['r2'],
): Promise<void> {
  const checksumsPath = join(absoluteBinariesDir, 'checksums.txt')
  if (!existsSync(checksumsPath)) {
    throw new Error('checksums.txt is required to build CLI manifest')
  }

  const manifest = buildCliReleaseManifest({
    version,
    filenames: releaseArchives,
    checksumsContent: readFileSync(checksumsPath, 'utf-8'),
    uploadPrefix,
  })
  const manifestPath = join(tmpdir(), `browseros-cli-manifest-${version}.json`)
  await writeFile(
    manifestPath,
    `${JSON.stringify(manifest, null, 2)}\n`,
    'utf-8',
  )

  const versionedKey = joinObjectKey(
    uploadPrefix,
    `v${version}`,
    'manifest.json',
  )
  const latestKey = joinObjectKey(uploadPrefix, 'latest', 'manifest.json')

  log.step('Uploading manifest.json')
  await uploadFileToObject(client, r2, versionedKey, manifestPath, {
    contentType: JSON_CONTENT_TYPE,
  })
  await uploadFileToObject(client, r2, latestKey, manifestPath, {
    contentType: JSON_CONTENT_TYPE,
  })
  log.success(`Uploaded ${latestKey}`)
  log.info(`${CDN_BASE_URL}/${latestKey}`)
}

async function uploadCliRelease(
  rootDir: string,
  options: CliReleaseOptions,
): Promise<void> {
  const { version, binariesDir } = options
  const absoluteBinariesDir = resolve(rootDir, binariesDir)

  if (!existsSync(absoluteBinariesDir)) {
    throw new Error(`Binaries directory not found: ${binariesDir}`)
  }

  const archives = readdirSync(absoluteBinariesDir).filter(
    (f) => f.endsWith('.tar.gz') || f.endsWith('.zip') || f === 'checksums.txt',
  )
  if (archives.length === 0) {
    throw new Error(`No archives found in ${binariesDir}`)
  }
  const releaseArchives = archives.filter((f) => f !== 'checksums.txt')

  const { r2 } = loadCliUploadConfig(rootDir)
  const client = createR2Client(r2)

  log.header(`Uploading BrowserOS CLI v${version} release`)

  try {
    for (const filename of archives) {
      const filePath = join(absoluteBinariesDir, filename)
      const versionedKey = joinObjectKey(
        r2.uploadPrefix,
        `v${version}`,
        filename,
      )
      const latestKey = joinObjectKey(r2.uploadPrefix, 'latest', filename)

      log.step(`Uploading ${filename}`)
      await uploadFileToObject(client, r2, versionedKey, filePath)
      await uploadFileToObject(client, r2, latestKey, filePath)
      log.success(`Uploaded ${filename}`)
      log.info(`${CDN_BASE_URL}/${versionedKey}`)
    }

    await uploadCliManifest(
      client,
      version,
      releaseArchives,
      r2.uploadPrefix,
      absoluteBinariesDir,
      r2,
    )

    const versionTxtPath = join(tmpdir(), 'browseros-cli-version.txt')
    await writeFile(versionTxtPath, version, 'utf-8')
    const versionKey = joinObjectKey(r2.uploadPrefix, 'latest', 'version.txt')
    await uploadFileToObject(client, r2, versionKey, versionTxtPath, {
      contentType: 'text/plain; charset=utf-8',
    })
    log.success(`Uploaded ${versionKey}`)
    log.info(`${CDN_BASE_URL}/${versionKey}`)

    log.done('CLI binary upload completed')
  } finally {
    client.destroy()
  }

  await uploadCliInstallers(rootDir)
}
