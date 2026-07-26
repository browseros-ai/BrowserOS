import { Route, Routes } from 'react-router'
import { OnboardingV2 } from '@/onboarding/OnboardingV2'

export function App() {
  return (
    <Routes>
      <Route path="*" element={<OnboardingV2 />} />
    </Routes>
  )
}
