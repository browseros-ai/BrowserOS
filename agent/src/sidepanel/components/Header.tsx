import React, { memo, useState } from 'react'
import { Button } from '@/sidepanel/components/ui/button'
import { useSidePanelPortMessaging } from '@/sidepanel/hooks'
import { MessageType } from '@/lib/types/messaging'
import { useAnalytics } from '../hooks/useAnalytics'
// import { HelpSection } from './HelpSection'
import { Pause, RotateCcw } from 'lucide-react'
import { useSettingsStore } from '@/sidepanel/stores/settingsStore'
import { useEffect } from 'react'
import { MCP_SERVERS, type MCPServerConfig } from '@/config/mcpServers'


interface HeaderProps {
  onReset: () => void
  showReset: boolean  // This now means "has messages to reset"
  isProcessing: boolean
}

/**
 * Header component for the sidepanel
 * Displays title, connection status, and action buttons (pause/reset)
 * Memoized to prevent unnecessary re-renders
 */
export const Header = memo(function Header({ onReset, showReset, isProcessing }: HeaderProps) {
  const { sendMessage, connected, addMessageListener, removeMessageListener } = useSidePanelPortMessaging()
  const { trackClick } = useAnalytics()
  
  const [showMCPDropdown, setShowMCPDropdown] = useState(false)
  const [mcpInstallStatus, setMcpInstallStatus] = useState<{ message: string; type: 'error' | 'success' } | null>(null)
  const [installedServers, setInstalledServers] = useState<any[]>([])
  const { theme } = useSettingsStore()
  
  const handleCancel = () => {
    trackClick('pause_task')
    sendMessage(MessageType.CANCEL_TASK, {
      reason: 'User clicked pause button',
      source: 'sidepanel'
    })
  }
  
  const handleReset = () => {
    trackClick('reset_conversation')
    // Send reset message to background
    sendMessage(MessageType.RESET_CONVERSATION, {
      source: 'sidepanel'
    })
    
    // Clear local state
    onReset()
  }



  const fetchInstalledServers = () => {
    sendMessage(MessageType.MCP_GET_INSTALLED_SERVERS, {})
  }

  // Close dropdown when clicking outside and fetch servers when opening
  useEffect(() => {
    if (!showMCPDropdown) return
    
    // Fetch installed servers when dropdown opens
    fetchInstalledServers()
    
    const handleClickOutside = (e: MouseEvent) => {
      const target = e.target as HTMLElement
      if (!target.closest('.mcp-dropdown-container')) {
        setShowMCPDropdown(false)
      }
    }
    
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [showMCPDropdown])

  

  // Load installed servers
  useEffect(() => {
    const handler = (payload: any) => {
      if (payload && payload.status === 'success' && payload.data) {
        // Handle installed servers response
        if (payload.data.servers) {
          setInstalledServers(payload.data.servers)
        }
      }
    }
    addMessageListener<any>(MessageType.WORKFLOW_STATUS, handler)
    return () => removeMessageListener<any>(MessageType.WORKFLOW_STATUS, handler)
  }, [])

  // Listen for MCP server installation/deletion status
  useEffect(() => {
    const handler = (payload: any) => {
      if (payload.status === 'success') {
        // Get server name from config for display
        const serverName = MCP_SERVERS.find(s => s.id === payload.serverId)?.name || payload.serverId
        setMcpInstallStatus({
          message: `${serverName} connected successfully!`,
          type: 'success'
        })
        // Refresh installed servers list after successful installation
        fetchInstalledServers()
      } else if (payload.status === 'deleted') {
        setMcpInstallStatus({
          message: 'Server removed successfully',
          type: 'success'
        })
        // Refresh installed servers list after successful deletion
        fetchInstalledServers()
      } else if (payload.status === 'auth_failed') {
        setMcpInstallStatus({
          message: payload.error || 'Authentication failed. Please try again.',
          type: 'error'
        })
      } else if (payload.status === 'error') {
        setMcpInstallStatus({
          message: payload.error || 'Operation failed. Please try again.',
          type: 'error'
        })
      }
      
      // Clear message after 5 seconds
      setTimeout(() => setMcpInstallStatus(null), 5000)
    }
    
    addMessageListener<any>(MessageType.MCP_SERVER_STATUS, handler)
    return () => removeMessageListener<any>(MessageType.MCP_SERVER_STATUS, handler)
  }, [])

  // Example work items that can be automated by the agent
  const EXAMPLE_WORK: string[] = [
    'Fill out and submit a web form',
    'Scan product listings and compare prices',
    'Extract contact info from a webpage',
    'Schedule meetings from email threads',
    'Auto-fill repetitive form fields',
    'Download invoices from account pages',
    'Monitor a page for price drops',
    'Collect article summaries from sites',
    'Open multiple tabs and capture screenshots',
    'Follow a link tree and list all URLs'
  ]

  const [currentExampleIndex, setCurrentExampleIndex] = useState<number>(Math.floor(Math.random() * EXAMPLE_WORK.length))
  // Rotate example every 10 seconds automatically
  useEffect(() => {
    const interval = setInterval(() => {
      setCurrentExampleIndex((prevIndex: number) => {
        const next = (prevIndex + 1) % EXAMPLE_WORK.length
        return next
      })
    }, 5000)
    return () => clearInterval(interval)
  }, [])
  return (
    <>
      <header 
        className="relative flex items-center justify-between h-12 px-3 bg-[#262626]/90 border-b border-white/10 backdrop-blur"
        role="banner"
      >
        


        {/* Left side - Control buttons */}
        <nav className="flex items-center gap-2 sm:gap-3" role="navigation" aria-label="Chat controls">
          {/* Show Pause button if processing */}
          {isProcessing && (
            <Button
              onClick={handleCancel}
              variant="ghost"
              size="sm"
              className="h-9 w-9 p-0 rounded-xl bg-white/5 hover:bg-red-500/20 text-white transition-all duration-300"
              aria-label="Pause current task"
              title="Pause"
            >
              <Pause className="w-4 h-4" />
            </Button>
          )}
          
          {/* Show Reset button if has messages */}
          {showReset && (
            <Button
              onClick={handleReset}
              variant="ghost"
              size="sm"
              className="h-9 w-9 p-0 rounded-xl bg-white/5 hover:bg-orange-500/20 text-white transition-all duration-300"
              aria-label="Reset conversation"
              title="Reset"
            >
              <RotateCcw className="w-4 h-4" />
            </Button>
          )}
        </nav>

        {/* Right side - Examples (absolute to ensure visibility) */}
        <div className="absolute right-3 top-1/2 -translate-y-1/2">
          <div className="pointer-events-auto flex items-center gap-2 text-sm text-white/95  font-mono bg-[#00000040] px-2 py-1 rounded-md">
            <span
              className="text-sm text-white/95 whitespace-nowrap"
              title={EXAMPLE_WORK[currentExampleIndex]}
              role="status"
              aria-live="polite"
            >
              {EXAMPLE_WORK[currentExampleIndex]}
            </span>
          </div>
        </div>
      </header>
    </>
  )
})