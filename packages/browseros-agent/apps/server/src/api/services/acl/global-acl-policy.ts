import { mkdir, readFile, rename, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'
import type { AclRule } from '@browseros/shared/types/acl'
import { getBrowserosDir } from '../../../lib/browseros-dir'
import { logger } from '../../../lib/logger'

const ACL_RULES_FILE_NAME = 'acl-rules.json'

type StoredAclRules = {
  aclRules?: AclRule[]
}

function cloneRules(rules: AclRule[]): AclRule[] {
  return rules.map((rule) => ({ ...rule }))
}

export class GlobalAclPolicyService {
  private rules: AclRule[] = []

  readonly filePath = join(getBrowserosDir(), ACL_RULES_FILE_NAME)

  async load(): Promise<void> {
    try {
      const raw = await readFile(this.filePath, 'utf8')
      const parsed = JSON.parse(raw) as StoredAclRules
      this.rules = this.normalizeRules(parsed.aclRules ?? [])
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
        logger.warn('Failed to load global ACL rules, starting empty', {
          error: error instanceof Error ? error.message : String(error),
          filePath: this.filePath,
        })
      }
      this.rules = []
    }
  }

  getRules(): AclRule[] {
    return cloneRules(this.rules)
  }

  getEnabledRules(): AclRule[] {
    return cloneRules(this.rules.filter((rule) => rule.enabled))
  }

  async setRules(rules: AclRule[]): Promise<AclRule[]> {
    this.rules = this.normalizeRules(rules)
    await mkdir(dirname(this.filePath), { recursive: true })

    const tempPath = `${this.filePath}.tmp`
    const content = `${JSON.stringify({ aclRules: this.rules }, null, 2)}\n`
    await writeFile(tempPath, content, 'utf8')
    await rename(tempPath, this.filePath)

    return this.getRules()
  }

  private normalizeRules(rules: AclRule[]): AclRule[] {
    return cloneRules(rules)
  }
}
