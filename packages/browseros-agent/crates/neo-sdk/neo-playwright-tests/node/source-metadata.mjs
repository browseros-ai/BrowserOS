/** Identify a local checkout or an explicitly described remote snapshot, without cloning it. */
import { execFileSync } from 'node:child_process'
import fs from 'node:fs'
import path from 'node:path'

export function sourceMetadata(agent, environment = process.env) {
  const fields = [
    'NEO_SDK_SOURCE_ROOT',
    'NEO_SDK_SOURCE_REVISION',
    'NEO_SDK_SOURCE_DIRTY',
  ]
  if (fields.some((field) => environment[field] !== undefined)) {
    if (
      !fields.every((field) => environment[field]) ||
      !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(
        environment.NEO_SDK_SOURCE_REVISION,
      ) ||
      !['true', 'false'].includes(environment.NEO_SDK_SOURCE_DIRTY)
    )
      throw new Error(
        'Snapshot metadata requires a root, full revision and true/false dirty flag',
      )
    if (
      fs.realpathSync(
        path.join(environment.NEO_SDK_SOURCE_ROOT, 'packages/browseros-agent'),
      ) !== fs.realpathSync(agent)
    )
      throw new Error('Snapshot root does not contain the spec runner')
    return {
      revision: environment.NEO_SDK_SOURCE_REVISION,
      dirty: environment.NEO_SDK_SOURCE_DIRTY === 'true',
    }
  }
  const git = (...args) =>
    execFileSync('git', args, { cwd: agent, encoding: 'utf8' }).trim()
  return {
    revision: git('rev-parse', 'HEAD'),
    dirty: Boolean(git('status', '--porcelain', '--', agent)),
  }
}
