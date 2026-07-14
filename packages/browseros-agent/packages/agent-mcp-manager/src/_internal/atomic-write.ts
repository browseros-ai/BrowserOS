import * as fsp from 'node:fs/promises'
import * as path from 'node:path'

/**
 * Write `data` to `file` via a sibling temp file + rename. Creates the
 * parent directory if needed. The rename is atomic on POSIX
 * filesystems; cross-platform "good enough" elsewhere.
 */
export async function atomicWriteFile(
  file: string,
  data: string,
): Promise<void> {
  await fsp.mkdir(path.dirname(file), { recursive: true })
  const tmp = `${file}.tmp-${process.pid}-${Date.now()}`
  try {
    await fsp.writeFile(tmp, data, 'utf8')
    await fsp.rename(tmp, file)
  } catch (err) {
    // Best-effort cleanup of the temp file on failure.
    try {
      await fsp.unlink(tmp)
    } catch {
      // ignore
    }
    throw err
  }
}

/**
 * Read a file's contents plus a flag saying whether the file existed on
 * disk. An empty file on disk still returns `exists: true`; only ENOENT
 * returns `exists: false`. Used by `readState` so `AgentFileState.exists`
 * matches the JSDoc contract instead of conflating "existed" with
 * "non-empty".
 */
export async function readFileWithExistence(
  file: string,
): Promise<{ content: string; exists: boolean }> {
  try {
    const content = await fsp.readFile(file, 'utf8')
    return { content, exists: true }
  } catch (err) {
    if ((err as NodeJS.ErrnoException).code === 'ENOENT') {
      return { content: '', exists: false }
    }
    throw err
  }
}
