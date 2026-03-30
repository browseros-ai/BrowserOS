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
