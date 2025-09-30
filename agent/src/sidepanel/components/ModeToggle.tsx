import React from 'react'
import { useSettingsStore } from '@/sidepanel/stores/settingsStore'

/**
 * ModeToggle - Toggle between Chat Mode (Q&A) and Agent Mode (automation)
 * Inspired by the Write/Chat toggle design
 */
export function ModeToggle() {
  const { chatMode, setChatMode } = useSettingsStore()

  return (
    <div className='flex items-center'>
      {/* Use design tokens via CSS variables to auto-adapt across light and dark */}
      <div className='inline-flex h-[25px] items-center gap-[2px] rounded-2xl border border-white/10 bg-[#1f1f1f]/80 p-[2px] backdrop-blur'>
        <button
          className={`h-[21px] px-3 rounded-xl text-[12px] font-semibold transition-colors ${chatMode ? 'bg-white/10 text-white border border-white/20' : 'text-white/60 hover:bg-white/5'}`}
          onClick={() => setChatMode(true)}
          aria-label='Chat mode for Q&A'
          title='Chat mode - Simple Q&A about pages'
        >
          Chat Mode
        </button>
        <button
          className={`h-[21px] px-3 rounded-xl text-[12px] font-semibold transition-colors ${!chatMode ? 'bg-white/10 text-white border border-white/20' : 'text-white/60 hover:bg-white/5'}`}
          onClick={() => setChatMode(false)}
          aria-label='Agent mode for automation'
          title='Agent mode - Complex web navigation tasks'
        >
          Agent Mode
        </button>
      </div>
    </div>
  )
}