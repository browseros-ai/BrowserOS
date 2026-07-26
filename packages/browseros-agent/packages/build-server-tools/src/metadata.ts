import { createHash } from 'node:crypto'
import { readdir, readFile, writeFile } from 'node:fs/promises'
import { join, relative, sep } from 'node:path'

interface MetadataFile {
  path: string
  sha256: string
  size: number
}

async function collectFiles(
  rootDir: string,
  currentDir: string,
): Promise<string[]> {
  const entries = await readdir(currentDir, { withFileTypes: true })
  const files: string[] = []

  for (const entry of entries) {
    const absolutePath = join(currentDir, entry.name)
    if (entry.isDirectory()) {
      const nested = await collectFiles(rootDir, absolutePath)
      files.push(...nested)
      continue
    }
    files.push(relative(rootDir, absolutePath))
  }

  return files
}

async function toMetadataFile(
  rootDir: string,
  filePath: string,
): Promise<MetadataFile> {
  const absolutePath = join(rootDir, filePath)
  const content = await readFile(absolutePath)
  return {
    path: filePath.split(sep).join('/'),
    sha256: createHash('sha256').update(content).digest('hex'),
    size: content.byteLength,
  }
}

export async function writeArtifactMetadata(
  artifactRoot: string,
  targetId: string,
  version: string,
): Promise<string> {
  const fileList = await collectFiles(artifactRoot, artifactRoot)
  const files: MetadataFile[] = []

  for (const filePath of fileList.sort()) {
    files.push(await toMetadataFile(artifactRoot, filePath))
  }

  const metadataPath = join(artifactRoot, 'artifact-metadata.json')
  await writeFile(
    metadataPath,
    JSON.stringify(
      {
        version,
        target: targetId,
        generatedAt: new Date().toISOString(),
        files,
      },
      null,
      2,
    ),
  )

  return metadataPath
}
