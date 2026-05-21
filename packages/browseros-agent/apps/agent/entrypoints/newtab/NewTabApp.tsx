import type { FC } from 'react'
import { HashRouter, Navigate, Route, Routes } from 'react-router'
import { Feature } from '@/lib/browseros/capabilities'
import { useCapabilities } from '@/lib/browseros/useCapabilities'
import { MemoryPage } from '../app/memory/MemoryPage'
import { SkillsPage } from '../app/skills/SkillsPage'
import { SoulPage } from '../app/soul/SoulPage'
import { NewTab } from '../newtab/index/NewTab'
import { NewTabLayout } from '../newtab/layout/NewTabLayout'
import { Personalize } from '../newtab/personalize/Personalize'

const NewTabApp: FC = () => {
  const { supports } = useCapabilities()
  const alphaEnabled = supports(Feature.ALPHA_FEATURES_SUPPORT)

  return (
    <HashRouter>
      <Routes>
        <Route
          path="home"
          element={<NewTabLayout useChatSessionOnHome={false} />}
        >
          {alphaEnabled ? (
            <Route path="personalize" element={<Personalize />} />
          ) : (
            <Route index element={<NewTab />} />
          )}
          <Route path="soul" element={<SoulPage />} />
          <Route path="skills" element={<SkillsPage />} />
          <Route path="memory" element={<MemoryPage />} />
        </Route>
        <Route path="*" element={<Navigate to="/home" replace />} />
      </Routes>
    </HashRouter>
  )
}

export { NewTabApp }
