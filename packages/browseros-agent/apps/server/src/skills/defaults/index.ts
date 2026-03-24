import comparePrices from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/compare-prices/SKILL.md' with {
  type: 'text',
}
import deepResearch from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/deep-research/SKILL.md' with {
  type: 'text',
}
import extractData from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/extract-data/SKILL.md' with {
  type: 'text',
}
import fillForm from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/fill-form/SKILL.md' with {
  type: 'text',
}
import findAlternatives from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/find-alternatives/SKILL.md' with {
  type: 'text',
}
import manageBookmarks from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/manage-bookmarks/SKILL.md' with {
  type: 'text',
}
import monitorPage from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/monitor-page/SKILL.md' with {
  type: 'text',
}
import organizeTabs from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/organize-tabs/SKILL.md' with {
  type: 'text',
}
import readLater from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/read-later/SKILL.md' with {
  type: 'text',
}
import savePage from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/save-page/SKILL.md' with {
  type: 'text',
}
import screenshotWalkthrough from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/screenshot-walkthrough/SKILL.md' with {
  type: 'text',
}
import summarizePage from '../../../../../vendor/sup-agent/apps/server/src/skills/defaults/summarize-page/SKILL.md' with {
  type: 'text',
}

type DefaultSkill = { id: string; content: string }

export const DEFAULT_SKILLS: DefaultSkill[] = [
  { id: 'summarize-page', content: summarizePage },
  { id: 'deep-research', content: deepResearch },
  { id: 'extract-data', content: extractData },
  { id: 'fill-form', content: fillForm },
  { id: 'screenshot-walkthrough', content: screenshotWalkthrough },
  { id: 'organize-tabs', content: organizeTabs },
  { id: 'compare-prices', content: comparePrices },
  { id: 'find-alternatives', content: findAlternatives },
  { id: 'save-page', content: savePage },
  { id: 'monitor-page', content: monitorPage },
  { id: 'read-later', content: readLater },
  { id: 'manage-bookmarks', content: manageBookmarks },
]
