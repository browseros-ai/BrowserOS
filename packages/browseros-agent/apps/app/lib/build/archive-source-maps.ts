import { mkdir, readdir, rename, rm } from 'node:fs/promises'
import { basename, dirname, join, relative, sep } from 'node:path'
import type { BuildOutput, ResolvedConfig } from 'wxt'

/** Keeps complete production maps for debugging outside the packaged extension. */
export async function archiveSourceMaps(
  config: Pick<ResolvedConfig, 'mode' | 'outDir' | 'outBaseDir'>,
  output: Readonly<BuildOutput>,
): Promise<void> {
  if (config.mode !== 'production') return

  const maps = (
    await readdir(config.outDir, { recursive: true, withFileTypes: true })
  ).filter((entry) => entry.isFile() && entry.name.endsWith('.map'))
  const archiveDir = join(
    config.outBaseDir,
    'sourcemaps',
    basename(config.outDir),
  )
  await rm(archiveDir, { recursive: true, force: true })
  await mkdir(archiveDir, { recursive: true })

  const movedFiles = new Set<string>()
  for (const entry of maps) {
    const sourcePath = join(entry.parentPath, entry.name)
    const relativePath = relative(config.outDir, sourcePath)
    const destinationPath = join(archiveDir, relativePath)
    await mkdir(dirname(destinationPath), { recursive: true })
    await rename(sourcePath, destinationPath)
    movedFiles.add(relativePath.split(sep).join('/'))
  }

  // WXT stats these files after build:done. Keep its mutable lists consistent
  // with the moved files so the summary never tries to read a missing map.
  for (const files of [
    output.publicAssets,
    ...output.steps.map((step) => step.chunks),
  ]) {
    for (let index = files.length - 1; index >= 0; index--) {
      if (movedFiles.has(files[index].fileName)) files.splice(index, 1)
    }
  }
}
