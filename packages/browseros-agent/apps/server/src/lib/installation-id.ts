/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */
import { randomUUID } from 'node:crypto'
import { link, mkdir, open, readFile, unlink } from 'node:fs/promises'
import { join } from 'node:path'

const INSTALLATION_FILE_NAME = 'installation.json'
const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

interface InstallationFile {
  install_id: string
}

/**
 * Loads the product-wide installation UUID, creating it when absent.
 *
 * Chromium and the sidecar can start concurrently, so creation publishes a
 * complete temporary file with a no-clobber hard link. Every losing process
 * then adopts the winner's ID instead of splitting one install across IDs.
 */
export async function loadOrCreateInstallationId(
  productStateDirectory: string,
): Promise<string> {
  const installationPath = join(productStateDirectory, INSTALLATION_FILE_NAME)
  try {
    return await readInstallationId(installationPath)
  } catch (error) {
    if (!isNotFoundError(error)) throw error
  }

  await mkdir(productStateDirectory, { recursive: true })
  const candidateId = randomUUID()
  const temporaryPath = join(
    productStateDirectory,
    `.${INSTALLATION_FILE_NAME}.${process.pid}.${randomUUID()}.tmp`,
  )
  const contents = `${JSON.stringify({ install_id: candidateId } satisfies InstallationFile, null, 2)}\n`

  const temporaryFile = await open(temporaryPath, 'wx', 0o600)
  try {
    try {
      await temporaryFile.writeFile(contents, 'utf8')
      await temporaryFile.sync()
    } finally {
      await temporaryFile.close()
    }
    try {
      await link(temporaryPath, installationPath)
      return candidateId
    } catch (error) {
      if (!isAlreadyExistsError(error)) throw error
      return await readInstallationId(installationPath)
    }
  } finally {
    await unlink(temporaryPath).catch(() => undefined)
  }
}

async function readInstallationId(path: string): Promise<string> {
  const raw = await readFile(path, 'utf8')
  let value: unknown
  try {
    value = JSON.parse(raw)
  } catch (error) {
    throw new Error(`Invalid installation identity file: ${path}`, {
      cause: error,
    })
  }

  const installId =
    typeof value === 'object' && value !== null && 'install_id' in value
      ? (value as Partial<InstallationFile>).install_id
      : undefined
  if (typeof installId !== 'string' || !UUID_PATTERN.test(installId)) {
    throw new Error(`Invalid installation identity file: ${path}`)
  }
  return installId
}

function isNotFoundError(error: unknown): boolean {
  return isNodeError(error, 'ENOENT')
}

function isAlreadyExistsError(error: unknown): boolean {
  return isNodeError(error, 'EEXIST')
}

function isNodeError(error: unknown, code: string): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    error.code === code
  )
}
