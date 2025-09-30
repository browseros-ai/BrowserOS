import React, { useEffect, useState } from 'react'
import { CommandInput } from './components/CommandInput'
import { ThemeToggle } from './components/ThemeToggle'
import { UserAgentsSection } from './components/UserAgentsSection'
import { ShootingStars } from './components/shooting-stars'
import { StarsBackground } from './components/stars-background'
import { useSettingsStore } from '@/sidepanel/stores/settingsStore'
import { useAgentsStore } from './stores/agentsStore'
import { Settings } from 'lucide-react'

export function NewTab() {
  const { theme, fontSize } = useSettingsStore()
  const [isSettingsOpen, setIsSettingsOpen] = useState(false)
  const { loadAgents } = useAgentsStore()
  
  // Load agents from storage on mount
  useEffect(() => {
    // Load agents from storage
    chrome.storage.local.get('agents', (result) => {
      if (result.agents) {
        loadAgents(result.agents)
      }
    })
  }, [loadAgents])
  
  // Apply theme and font size
  useEffect(() => {
    document.documentElement.style.setProperty('--app-font-size', `${fontSize}px`)
    const root = document.documentElement
    root.classList.remove('dark')
    if (theme === 'dark') root.classList.add('dark')
  }, [theme, fontSize])
  
  return (
    <div className="min-h-screen bg-background relative">
      {/* Galaxy Background with Colorful Stars */}
      <div className="fixed inset-0 bg-black dark:bg-black pointer-events-none z-0">
        <StarsBackground
          starDensity={0.0002}
          allStarsTwinkle={true}
          twinkleProbability={0.8}
          minTwinkleSpeed={0.3}
          maxTwinkleSpeed={1.2}
          className="opacity-90"
        />
      </div>

      {/* Shooting Stars */}
      <ShootingStars
        minSpeed={5}
        maxSpeed={10}
        minDelay={2000}
        maxDelay={6000}
        starWidth={12}
        starHeight={2}
        className="pointer-events-none z-10"
      />

      {/* Top Right Controls - Settings and Theme Toggle */}
      <div className="absolute top-6 right-6 z-30 flex items-center gap-2">
        {/* Settings Button */}


        {/* Theme Toggle */}
        <ThemeToggle />
      </div>

      {/* Main Content - Centered (slightly above center for better visual balance) */}
      <div className="min-h-screen flex flex-col items-center justify-center relative z-20">
        <div className="w-full max-w-3xl px-4 -mt-20">
          {/* Nemo Branding */}
          <div className="flex items-center justify-center mb-10">
            {/* <img 
              src="/assets/nemo.svg" 
              alt="Nemo" 
              className="w-12 h-12 mr-3"
            /> */}
            <span className="text-5xl font-geist-mono text-foreground tracking-wider bg-clip-text text-transparent bg-gradient-to-r from-purple-500 via-pink-500 to-indigo-500 hover:scale-105 transition-transform duration-200">
              Nemo
            </span>
          </div>
          
          {/* Command Input - Clean and Centered */}
          <CommandInput />
        </div>
        
        {/* User Agents Section - Shows up to 4 random agents */}
        <UserAgentsSection />
      </div>
    </div>
  )
}