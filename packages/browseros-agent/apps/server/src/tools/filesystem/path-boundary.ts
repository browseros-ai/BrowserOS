import { lstat, realpath } from 'node:fs/promises'
import { dirname, isAbsolute, relative, resolve, win32 } from 'node:path'
import { getBrowserosDir, getToolOutputDir } from '../../lib/browseros-dir'

function isAbsoluteInput(inputPath: string): boolean {
  return isAbsolute(inputPath) || win32.isAbsolute(inputPath)
}

export function isPathInside(root: string, candidate: string): boolean {
  const rel = relative(root, candidate)
  const escapesRoot =
    rel === '..' || rel.startsWith('../') || rel.startsWith('..\\')
  return rel === '' || (!escapesRoot && !isAbsoluteInput(rel))
}

export async function isBrowserosStatePath(
  inputPath: string,
): Promise<boolean> {
  if (!isAbsoluteInput(inputPath)) return false

  const stateRoot = resolve(getBrowserosDir())
  const candidate = resolve(inputPath)
  if (isPathInside(stateRoot, candidate)) return true

  const realStateRoot = await realpath(stateRoot).catch(() => null)
  const realCandidate = await realpath(candidate).catch(() => null)
  return Boolean(
    realStateRoot &&
      realCandidate &&
      isPathInside(realStateRoot, realCandidate),
  )
}

function assertRelativeWorkspaceInput(inputPath: string): void {
  if (isAbsoluteInput(inputPath)) {
    throw new Error('Path must be relative to the selected workspace.')
  }
}

function assertAbsoluteBrowserosOutputInput(inputPath: string): void {
  if (!isAbsoluteInput(inputPath)) {
    throw new Error('Path must be an absolute BrowserOS tool output path.')
  }
}

function assertInsideWorkspace(root: string, candidate: string): void {
  if (!isPathInside(root, candidate)) {
    throw new Error('Path is outside the selected workspace.')
  }
}

export async function resolveWorkspaceRoot(cwd: string): Promise<string> {
  return await realpath(cwd)
}

async function findExistingParent(
  root: string,
  targetPath: string,
): Promise<string> {
  let parent = dirname(targetPath)

  while (isPathInside(root, parent)) {
    try {
      await lstat(parent)
      return parent
    } catch (error) {
      if (!(error instanceof Error) || !('code' in error)) throw error
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
    }

    const next = dirname(parent)
    if (next === parent) break
    parent = next
  }

  throw new Error('Path is outside the selected workspace.')
}

/** Resolves an existing workspace path and rejects traversal or symlink escapes. */
export async function resolveWorkspacePath(
  cwd: string,
  inputPath: string,
): Promise<string> {
  return await resolveWorkspacePathFromRoot(
    await resolveWorkspaceRoot(cwd),
    inputPath,
  )
}

/** Resolves an existing workspace path when the canonical workspace root is already known. */
export async function resolveWorkspacePathFromRoot(
  root: string,
  inputPath: string,
): Promise<string> {
  assertRelativeWorkspaceInput(inputPath)
  const candidate = resolve(root, inputPath)
  assertInsideWorkspace(root, candidate)
  const canonical = await realpath(candidate)
  assertInsideWorkspace(root, canonical)
  return canonical
}

/** Resolves a workspace write target, validating the existing parent chain first. */
export async function resolveWorkspaceWritePath(
  cwd: string,
  inputPath: string,
): Promise<string> {
  assertRelativeWorkspaceInput(inputPath)
  const root = await resolveWorkspaceRoot(cwd)
  const candidate = resolve(root, inputPath)
  assertInsideWorkspace(root, candidate)

  try {
    const canonical = await realpath(candidate)
    assertInsideWorkspace(root, canonical)
    return canonical
  } catch (error) {
    if (!(error instanceof Error) || !('code' in error)) throw error
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
  }

  const parent = await findExistingParent(root, candidate)
  const canonicalParent = await realpath(parent)
  assertInsideWorkspace(root, canonicalParent)
  const resolved = resolve(canonicalParent, relative(parent, candidate))
  assertInsideWorkspace(root, resolved)
  return resolved
}

/** Resolves a BrowserOS-generated output file without exposing sibling app state. */
export async function resolveBrowserToolOutputPath(
  inputPath: string,
): Promise<string> {
  assertAbsoluteBrowserosOutputInput(inputPath)
  const outputRoot = await getToolOutputDir()
  const candidate = resolve(inputPath)
  const canonical = await realpath(candidate)
  if (!isPathInside(outputRoot, canonical)) {
    throw new Error('Path is outside BrowserOS tool output.')
  }
  return canonical
}
