import { act } from './act'
import { diff } from './diff'
import type { ToolDefinition } from './framework'
import { grep } from './grep'
import { navigate } from './navigate'
import { read } from './read'
import { run } from './run'
import { screenshot } from './screenshot'
import { snapshot } from './snapshot'
import { tabs } from './tabs'
import { upload } from './upload'
import { wait } from './wait'

/** The complete browser tool surface, in a sensible discovery order. */
export const BROWSER_TOOLS: readonly ToolDefinition[] = [
  tabs,
  navigate,
  snapshot,
  diff,
  act,
  upload,
  read,
  grep,
  screenshot,
  wait,
  run,
]
