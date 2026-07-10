import { mkdir, mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

export interface TmpWorkspace {
  root: string
  workspaceDir: string
  /** `${root}/configs` — point agent config files in here. */
  configsDir: string
  cleanup(): Promise<void>
}

export async function makeTmpWorkspace(): Promise<TmpWorkspace> {
  const root = await mkdtemp(join(tmpdir(), 'mcp-mgr-'))
  const workspaceDir = join(root, 'workspace')
  const configsDir = join(root, 'configs')
  await mkdir(workspaceDir, { recursive: true })
  await mkdir(configsDir, { recursive: true })
  return {
    root,
    workspaceDir,
    configsDir,
    async cleanup() {
      await rm(root, { recursive: true, force: true })
    },
  }
}
