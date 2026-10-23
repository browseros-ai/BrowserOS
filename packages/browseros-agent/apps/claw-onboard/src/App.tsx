import { Route, Routes } from 'react-router'
import { OnboardingChat } from '@/onboarding/OnboardingChat'

export function App() {
  return (
    <Routes>
      <Route path="*" element={<OnboardingChat />} />
    </Routes>
  )
}
