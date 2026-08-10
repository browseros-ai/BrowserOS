import { createHash } from 'node:crypto'
import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname } from 'node:path'

import {
  GetObjectCommand,
  PutObjectCommand,
  S3Client,
  type S3ClientConfig,
} from '@aws-sdk/client-s3'

import type { R2Config } from './types'

export interface UploadFileOptions {
  contentType?: string
}

export interface ImmutableObjectIdentity {
  component: string
  releaseSha: string
  target: string
  version: string
}

export interface ImmutableUploadOptions extends UploadFileOptions {
  identity?: ImmutableObjectIdentity
}

export type ImmutableUploadResult = 'uploaded' | 'recovered' | 'reused'

interface ExistingObject {
  bytes: Buffer
  metadata: Record<string, string>
}

function createClientConfig(r2: R2Config): S3ClientConfig {
  return {
    region: 'auto',
    endpoint: `https://${r2.accountId}.r2.cloudflarestorage.com`,
    credentials: {
      accessKeyId: r2.accessKeyId,
      secretAccessKey: r2.secretAccessKey,
    },
  }
}

function trimEdgeSlashes(value: string): string {
  return value.replace(/^\/+|\/+$/g, '')
}

export function joinObjectKey(...parts: string[]): string {
  return parts
    .map((part) => trimEdgeSlashes(part))
    .filter((part) => part.length > 0)
    .join('/')
}

export function createR2Client(r2: R2Config): S3Client {
  return new S3Client(createClientConfig(r2))
}

async function readBodyToBuffer(body: unknown): Promise<Buffer> {
  const withTransform = body as {
    transformToByteArray?: () => Promise<Uint8Array>
  }
  if (withTransform?.transformToByteArray) {
    const bytes = await withTransform.transformToByteArray()
    return Buffer.from(bytes)
  }

  const readable = body as AsyncIterable<Uint8Array>
  if (!readable || typeof readable[Symbol.asyncIterator] !== 'function') {
    throw new Error('Cloudflare R2 response body is not readable')
  }

  const chunks: Buffer[] = []
  for await (const chunk of readable) {
    chunks.push(Buffer.from(chunk))
  }
  return Buffer.concat(chunks)
}

export async function downloadObjectToFile(
  client: S3Client,
  r2: R2Config,
  key: string,
  destinationPath: string,
): Promise<void> {
  const objectKey = joinObjectKey(r2.downloadPrefix, key)
  const response = await client.send(
    new GetObjectCommand({
      Bucket: r2.bucket,
      Key: objectKey,
    }),
  )

  const bytes = await readBodyToBuffer(response.Body)
  await mkdir(dirname(destinationPath), { recursive: true })
  await writeFile(destinationPath, bytes)
}

export async function uploadFileToObject(
  client: S3Client,
  r2: R2Config,
  key: string,
  filePath: string,
  options: UploadFileOptions = {},
): Promise<void> {
  const data = await readFile(filePath)
  await client.send(
    new PutObjectCommand({
      Bucket: r2.bucket,
      Key: key,
      Body: data,
      ContentType: options.contentType ?? 'application/zip',
    }),
  )
}

function isMissingObject(error: unknown): boolean {
  const candidate = error as {
    name?: string
    Code?: string
    $metadata?: { httpStatusCode?: number }
  }
  return (
    candidate.$metadata?.httpStatusCode === 404 ||
    candidate.name === 'NoSuchKey' ||
    candidate.Code === 'NoSuchKey'
  )
}

function isPreconditionFailure(error: unknown): boolean {
  const candidate = error as {
    name?: string
    Code?: string
    $metadata?: { httpStatusCode?: number }
  }
  return (
    candidate.$metadata?.httpStatusCode === 409 ||
    candidate.$metadata?.httpStatusCode === 412 ||
    candidate.name === 'ConditionalRequestConflict' ||
    candidate.name === 'PreconditionFailed' ||
    candidate.Code === 'ConditionalRequestConflict' ||
    candidate.Code === 'PreconditionFailed'
  )
}

async function existingObject(
  client: S3Client,
  r2: R2Config,
  key: string,
): Promise<ExistingObject | null> {
  try {
    const response = await client.send(
      new GetObjectCommand({ Bucket: r2.bucket, Key: key }),
    )
    return {
      bytes: await readBodyToBuffer(response.Body),
      metadata: Object.fromEntries(
        Object.entries(response.Metadata ?? {}).map(([name, value]) => [
          name.toLowerCase(),
          value ?? '',
        ]),
      ),
    }
  } catch (error) {
    if (isMissingObject(error)) {
      return null
    }
    throw error
  }
}

function sha256(data: Buffer): string {
  return createHash('sha256').update(data).digest('hex')
}

function identityMetadata(
  identity: ImmutableObjectIdentity,
  data: Buffer,
): Record<string, string> {
  return {
    component: identity.component,
    'release-sha': identity.releaseSha,
    target: identity.target,
    version: identity.version,
    sha256: sha256(data),
  }
}

function validateExistingObject(
  existing: ExistingObject,
  key: string,
  identity: ImmutableObjectIdentity,
): void {
  const expected = identityMetadata(identity, existing.bytes)
  for (const [name, value] of Object.entries(expected)) {
    if (existing.metadata[name] !== value) {
      throw new Error(
        `immutable R2 object binding mismatch for ${key}: ${name}`,
      )
    }
  }
}

export async function recoverImmutableFileFromObject(
  client: S3Client,
  r2: R2Config,
  key: string,
  destinationPath: string,
  identity: ImmutableObjectIdentity,
): Promise<boolean> {
  const existing = await existingObject(client, r2, key)
  if (!existing) {
    return false
  }

  validateExistingObject(existing, key, identity)
  await mkdir(dirname(destinationPath), { recursive: true })
  await writeFile(destinationPath, existing.bytes)
  return true
}

export async function uploadImmutableFileToObject(
  client: S3Client,
  r2: R2Config,
  key: string,
  filePath: string,
  options: ImmutableUploadOptions = {},
): Promise<ImmutableUploadResult> {
  const data = await readFile(filePath)
  const existing = await existingObject(client, r2, key)
  if (existing) {
    if (options.identity) {
      validateExistingObject(existing, key, options.identity)
      if (!existing.bytes.equals(data)) {
        await writeFile(filePath, existing.bytes)
        return 'recovered'
      }
      return 'reused'
    }
    if (existing.bytes.equals(data)) {
      return 'reused'
    }
    throw new Error(
      `immutable R2 object already exists with different bytes: ${key}`,
    )
  }

  try {
    await client.send(
      new PutObjectCommand({
        Bucket: r2.bucket,
        Key: key,
        Body: data,
        ContentType: options.contentType ?? 'application/zip',
        IfNoneMatch: '*',
        Metadata: options.identity
          ? identityMetadata(options.identity, data)
          : undefined,
      }),
    )
    return 'uploaded'
  } catch (error) {
    if (!isPreconditionFailure(error)) {
      throw error
    }
  }

  const raced = await existingObject(client, r2, key)
  if (raced) {
    if (options.identity) {
      validateExistingObject(raced, key, options.identity)
      if (!raced.bytes.equals(data)) {
        await writeFile(filePath, raced.bytes)
        return 'recovered'
      }
      return 'reused'
    }
    if (raced.bytes.equals(data)) {
      return 'reused'
    }
  }
  throw new Error(
    `immutable R2 object already exists with different bytes: ${key}`,
  )
}
