import { act } from './act'
import { diff } from './diff'
import { download } from './download'
import { evaluate } from './evaluate'
import type { ToolDefinition } from './framework'
import { grep } from './grep'
import { navigate } from './navigate'
import { pdf } from './pdf'
import { read } from './read'
import { run } from './run'
import { screenshot } from './screenshot'
import { snapshot } from './snapshot'
import { tab_groups } from './tab-groups'
import { tabs } from './tabs'
import { upload } from './upload'
import { wait } from './wait'
import { windows } from './windows'

export const BROWSER_TOOLS: readonly ToolDefinition[] = [
  tabs,
  tab_groups,
  navigate,
  snapshot,
  diff,
  act,
  download,
  upload,
  read,
  grep,
  screenshot,
  pdf,
  wait,
  windows,
  evaluate,
  run,
]
