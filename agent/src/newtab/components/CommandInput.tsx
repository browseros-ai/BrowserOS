import React, { useState, useRef, useEffect, useCallback } from 'react'
import { useProviderStore } from '../stores/providerStore'
import { fetchGoogleSuggestions } from '../utils/googlePredictions'

// Define props schema using Zod
import { z } from 'zod'

// AI provider options for @ mentions
// Supports tab completion: typing "@chat" + Tab completes to "@chatgpt"
// Supports tab completion: typing "@c" + Tab shows dropdown for chatgpt/claude
// Supports tab completion: typing "@" + Tab shows all providers
const AI_PROVIDERS = [
  { id: 'chatgpt', name: 'ChatGPT', icon: './assets/new_tab_search/ChatGPT_logo-green-white.svg', color: 'text-green-400' },
  { id: 'claude', name: 'Claude', icon: './assets/new_tab_search/claude.svg', color: 'text-orange-400' },
  { id: 'gemini', name: 'Gemini', icon: './assets/new_tab_search/google.svg', color: 'text-blue-400' }
] as const

type AIProvider = typeof AI_PROVIDERS[number]

const CommandInputPropsSchema = z.object({
  // Add any props if needed in the future
})

type CommandInputProps = z.infer<typeof CommandInputPropsSchema>

export function CommandInput({}: CommandInputProps = {}) {
  const [value, setValue] = useState('')
  const [isFocused, setIsFocused] = useState(false)
  const [suggestions, setSuggestions] = useState<string[]>([])
  const [selectedSuggestionIndex, setSelectedSuggestionIndex] = useState(-1)
  const [isLoadingSuggestions, setIsLoadingSuggestions] = useState(false)
  const [isAIDropdownOpen, setIsAIDropdownOpen] = useState(false)
  const [selectedAIProviderIndex, setSelectedAIProviderIndex] = useState(-1)
  const inputRef = useRef<HTMLInputElement>(null)
  const debounceTimerRef = useRef<NodeJS.Timeout | null>(null)

  const { executeProviderAction, providers } = useProviderStore()

  // Debounced suggestion fetching
  useEffect(() => {
    // Clear previous debounce timer
    if (debounceTimerRef.current) {
      clearTimeout(debounceTimerRef.current)
    }

    // Set new debounce timer
    debounceTimerRef.current = setTimeout(async () => {
      if (value.length > 1) {
        setIsLoadingSuggestions(true)
        try {
          const fetchedSuggestions = await fetchGoogleSuggestions(value)
          setSuggestions(fetchedSuggestions.slice(0, 8)) // Limit to 8 suggestions
          setSelectedSuggestionIndex(-1) // Reset selection
        } catch (error) {
          console.error('Failed to fetch suggestions:', error)
          setSuggestions([])
        } finally {
          setIsLoadingSuggestions(false)
        }
      } else {
        setSuggestions([])
        setSelectedSuggestionIndex(-1)
      }
    }, 100) // 100ms debounce

    // Cleanup function
    return () => {
      if (debounceTimerRef.current) {
        clearTimeout(debounceTimerRef.current)
      }
    }
  }, [value])

  // Handle @ mention detection and AI dropdown
  useEffect(() => {
    const hasAtSymbol = value.includes('@')
    const atIndex = value.lastIndexOf('@')

    if (hasAtSymbol && atIndex === value.length - 1) {
      // @ is at the end of the input - show AI dropdown
      setIsAIDropdownOpen(true)
      setSelectedAIProviderIndex(-1)
    } else if (hasAtSymbol && atIndex < value.length - 1) {
      // @ is in the middle with text after it - don't auto-show dropdown
      // but allow it to be opened by tab completion or manual interaction
      // Don't set to false here as it might interfere with tab completion
      setSelectedAIProviderIndex(-1)
    } else {
      // No @ symbol - hide AI dropdown
      setIsAIDropdownOpen(false)
      setSelectedAIProviderIndex(-1)
    }
  }, [value])

  // Auto-focus on mount with retry
  useEffect(() => {
    const focusInput = () => {
      if (inputRef.current) {
        inputRef.current.focus()
        // Ensure focus is maintained
        setTimeout(() => {
          if (inputRef.current && document.activeElement !== inputRef.current) {
            inputRef.current.focus()
          }
        }, 100)
      }
    }

    focusInput()
  }, [])

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault()
    if (!value.trim()) return

    const query = value.trim()

    // Check if this is an @ mention query
    if (query.includes('@')) {
      await handleAIMentionSubmit(query)
      return
    }

    console.log('CommandInput handleSubmit:', { query })

    // Execute using the default Nemo agent
    try {
      const nemoAgent = {
        id: 'nemo-agent',
        name: 'Nemo Agent',
        category: 'llm' as const,
        actionType: 'sidepanel' as const,
        available: true,
        iconUrl: '/assets/new_tab_search/nemo.svg'
      }
      await executeProviderAction(nemoAgent, query)
      console.log('Executed query:', query)
    } catch (error) {
      console.error('Failed to execute query:', error)
    }

    setValue('')
  }

  // Handle @ mention submissions (e.g., "explain quantum physics @chatgpt")
  const handleAIMentionSubmit = async (fullQuery: string) => {
    // Find the last @ symbol and split the query
    const atIndex = fullQuery.lastIndexOf('@')
    if (atIndex === -1 || atIndex === fullQuery.length - 1) return

    const queryText = fullQuery.substring(0, atIndex).trim()
    const providerName = fullQuery.substring(atIndex + 1).trim().toLowerCase()

    if (!queryText || !providerName) return

    console.log('AI mention detected:', { queryText, providerName })

    // Find the provider in the store
    const provider = providers.find(p =>
      p.id === providerName ||
      p.name.toLowerCase() === providerName ||
      p.name.toLowerCase().includes(providerName)
    )

    if (provider) {
      try {
        await executeProviderAction(provider, queryText)
        console.log('Executed AI mention:', { provider: provider.name, query: queryText })
      } catch (error) {
        console.error('Failed to execute AI mention:', error)
      }
    } else {
      console.warn('AI provider not found:', providerName)
    }

    setValue('')
  }
  
  // Handle suggestion selection
  const selectSuggestion = useCallback((suggestion: string) => {
    setValue(suggestion)

    // Update contentEditable directly for immediate feedback
    if (inputRef.current) {
      // Clear current content and set new content
      inputRef.current.textContent = suggestion

      // Position cursor at the end of the selected suggestion
      const selection = window.getSelection()
      if (selection) {
        // Create a new range and select the end position
        const range = document.createRange()
        range.selectNodeContents(inputRef.current)
        range.collapse(false) // false = collapse to end
        selection.removeAllRanges()
        selection.addRange(range)
      }
    }

    setSuggestions([])
    setSelectedSuggestionIndex(-1)
    // Focus after a brief delay to ensure cursor position is set
    setTimeout(() => inputRef.current?.focus(), 0)
  }, [])

  // Handle AI provider selection
  const selectAIProvider = useCallback((provider: AIProvider) => {
    // Replace @ with the selected provider name
    const newValue = value.replace(/@$/, `@${provider.name.toLowerCase()}`)
    setValue(newValue)

    // Update contentEditable directly for immediate feedback
    if (inputRef.current) {
      // Clear current content and set new content
      inputRef.current.textContent = newValue

      // Position cursor at the end of the selected provider
      const selection = window.getSelection()
      if (selection) {
        // Create a new range and select the end position
        const range = document.createRange()
        range.selectNodeContents(inputRef.current)
        range.collapse(false) // false = collapse to end
        selection.removeAllRanges()
        selection.addRange(range)
      }
    }

    setIsAIDropdownOpen(false)
    setSelectedAIProviderIndex(-1)
    // Focus after a brief delay to ensure cursor position is set
    setTimeout(() => inputRef.current?.focus(), 0)
  }, [value])

  // Handle keyboard navigation
  const handleKeyDown = useCallback((e: React.KeyboardEvent<HTMLInputElement>) => {
    // Handle Tab completion for @ mentions
    if (e.key === 'Tab') {
      const atIndex = value.lastIndexOf('@')
      if (atIndex !== -1 && atIndex < value.length) {
        e.preventDefault()

        // If @ is at the end, show all providers
        if (atIndex === value.length - 1) {
          setIsAIDropdownOpen(true)
          setSelectedAIProviderIndex(0)
          return
        }

        // If there's text after @, try to complete the provider name
        if (atIndex < value.length - 1) {
          const partialProvider = value.substring(atIndex + 1).toLowerCase()
          const matchingProviders = AI_PROVIDERS.filter(provider =>
            provider.id.startsWith(partialProvider) ||
            provider.name.toLowerCase().startsWith(partialProvider) ||
            partialProvider.startsWith(provider.id) ||
            partialProvider.startsWith(provider.name.toLowerCase())
          )

          if (matchingProviders.length === 1) {
            // Complete with the full provider name (e.g., "@chat" -> "@chatgpt")
            const completedValue = value.substring(0, atIndex + 1) + matchingProviders[0].id
            setValue(completedValue)

            // Update contentEditable directly for immediate feedback
            if (inputRef.current) {
              inputRef.current.textContent = completedValue

              // Position cursor at the end of the completed text
              const selection = window.getSelection()
              if (selection) {
                selection.collapse(inputRef.current, completedValue.length)
              }
            }

            setIsAIDropdownOpen(false)
            setSelectedAIProviderIndex(-1)
          } else if (matchingProviders.length > 1) {
            // Show dropdown for multiple matches (e.g., "@c" matches both chatgpt and claude)
            setIsAIDropdownOpen(true)
            setSelectedAIProviderIndex(0)
          } else if (partialProvider.length > 0) {
            // If no matches but there's partial text, show all providers
            setIsAIDropdownOpen(true)
            setSelectedAIProviderIndex(0)
          }
        }
      }
      return
    }

    // Handle AI provider dropdown navigation when @ is at the end
    if (isAIDropdownOpen && value.endsWith('@')) {
      switch (e.key) {
        case 'ArrowDown':
          e.preventDefault()
          setSelectedAIProviderIndex(prev =>
            prev < AI_PROVIDERS.length - 1 ? prev + 1 : 0
          )
          break
        case 'ArrowUp':
          e.preventDefault()
          setSelectedAIProviderIndex(prev =>
            prev > 0 ? prev - 1 : AI_PROVIDERS.length - 1
          )
          break
        case 'Enter':
          e.preventDefault()
          if (selectedAIProviderIndex >= 0 && selectedAIProviderIndex < AI_PROVIDERS.length) {
            selectAIProvider(AI_PROVIDERS[selectedAIProviderIndex])
          }
          break
        case 'Escape':
          setIsAIDropdownOpen(false)
          setSelectedAIProviderIndex(-1)
          break
        default:
          // Let other keys through to potentially trigger regular suggestions
          break
      }
      return
    }

    if (suggestions.length === 0) {
      if (e.key === 'Enter') {
        e.preventDefault()
        handleSubmit(e as any)
      }
      return
    }

    switch (e.key) {
      case 'ArrowDown':
        e.preventDefault()
        setSelectedSuggestionIndex(prev =>
          prev < suggestions.length - 1 ? prev + 1 : 0
        )
        break
      case 'ArrowUp':
        e.preventDefault()
        setSelectedSuggestionIndex(prev =>
          prev > 0 ? prev - 1 : suggestions.length - 1
        )
        break
      case 'Enter':
        e.preventDefault()
        if (selectedSuggestionIndex >= 0 && selectedSuggestionIndex < suggestions.length) {
          selectSuggestion(suggestions[selectedSuggestionIndex])
        } else {
          handleSubmit(e as any)
        }
        break
      case 'Escape':
        setSuggestions([])
        setSelectedSuggestionIndex(-1)
        break
      default:
        // For other keys, let the default behavior happen but reset selection
        setSelectedSuggestionIndex(-1)
        break
    }
  }, [suggestions, selectedSuggestionIndex, selectSuggestion, isAIDropdownOpen, selectedAIProviderIndex, value])

  // Simple placeholder
  const getPlaceholder = () => {
    return 'Ask me anything ...'
  }

  // Highlight @ mentions in the input
  const highlightAtSymbols = useCallback(() => {
    if (!inputRef.current) return

    const content = inputRef.current.textContent || ''
    if (!content.includes('@')) {
      // Remove previous highlights if no @ symbols
      const spans = inputRef.current.querySelectorAll('.at-highlight')
      spans.forEach(span => {
        const parent = span.parentNode
        if (parent) {
          parent.replaceChild(document.createTextNode(span.textContent || ''), span)
          parent.normalize()
        }
      })
      return
    }

    // Find all @ mentions and highlight them
    const mentionRegex = /@([a-zA-Z0-9]*)/g
    let match
    let hasChanges = false

    // Check if we need to make changes
    while ((match = mentionRegex.exec(content)) !== null) {
      const providerName = match[1]

      // Check if this is a complete, valid provider name that should be highlighted
      const isCompleteProvider = AI_PROVIDERS.some(provider =>
        provider.id === providerName.toLowerCase() ||
        provider.name.toLowerCase() === providerName.toLowerCase()
      )

      if (isCompleteProvider) {
        // Check if this mention is already highlighted
        const atIndex = match.index
        const mentionEnd = atIndex + match[0].length

        // Look for existing highlight at this position
        const textNode = inputRef.current.childNodes[0] as Text
        if (!textNode || textNode.textContent !== content) {
          hasChanges = true
          break
        }

        // Check if there's already a highlight span at this position
        let currentPos = 0
        let foundExistingHighlight = false

        for (let i = 0; i < inputRef.current.childNodes.length; i++) {
          const node = inputRef.current.childNodes[i]
          if (node.nodeType === Node.TEXT_NODE) {
            currentPos += node.textContent?.length || 0
          } else if (node.nodeType === Node.ELEMENT_NODE && (node as Element).classList.contains('at-highlight')) {
            if (currentPos === atIndex) {
              foundExistingHighlight = true
              break
            }
            currentPos += node.textContent?.length || 0
          }
        }

        if (!foundExistingHighlight) {
          hasChanges = true
          break
        }
      }
    }

    // If no changes needed, return early
    if (!hasChanges) return

    // Only rebuild DOM if there are actual changes to make
    const selection = window.getSelection()
    const cursorOffset = selection?.focusOffset || 0
    const cursorNode = selection?.focusNode

    // Calculate cursor position in the plain text content
    let textOffset = 0
    if (cursorNode && inputRef.current.contains(cursorNode)) {
      const treeWalker = document.createTreeWalker(
        inputRef.current,
        NodeFilter.SHOW_TEXT,
        null
      )
      let node
      while ((node = treeWalker.nextNode())) {
        if (node === cursorNode) {
          textOffset += cursorOffset
          break
        }
        textOffset += node.textContent?.length || 0
      }
    }

    // Clear current content
    inputRef.current.innerHTML = ''

    let lastIndex = 0

    // Find all @ mentions and highlight them
    mentionRegex.lastIndex = 0 // Reset regex
    while ((match = mentionRegex.exec(content)) !== null) {
      const mentionStart = match.index
      const mentionEnd = match.index + match[0].length
      const providerName = match[1]

      // Add text before the mention
      if (mentionStart > lastIndex) {
        const beforeText = content.substring(lastIndex, mentionStart)
        inputRef.current.appendChild(document.createTextNode(beforeText))
      }

      // Check if this is a complete, valid provider name
      const isCompleteProvider = AI_PROVIDERS.some(provider =>
        provider.id === providerName.toLowerCase() ||
        provider.name.toLowerCase() === providerName.toLowerCase()
      )

      // Highlight the mention only if it's a complete, valid provider name
      if (isCompleteProvider) {
        const highlightSpan = document.createElement('span')
        highlightSpan.className = 'at-highlight'
        highlightSpan.textContent = match[0]
        highlightSpan.style.backgroundColor = 'rgba(59, 130, 246, 0.3)' // Light blue background
        highlightSpan.style.borderRadius = '2px'
        highlightSpan.style.padding = '1px 2px'
        highlightSpan.style.margin = '0 1px'
        inputRef.current.appendChild(highlightSpan)
      } else {
        // Just add the @ symbol without highlighting
        inputRef.current.appendChild(document.createTextNode('@'))
      }

      lastIndex = mentionEnd
    }

    // Add any remaining text after the last mention
    if (lastIndex < content.length) {
      const remainingText = content.substring(lastIndex)
      inputRef.current.appendChild(document.createTextNode(remainingText))
    }

    // Restore cursor position only if we made changes
    if (selection && inputRef.current && hasChanges) {
      try {
        let currentOffset = 0
        const treeWalker = document.createTreeWalker(
          inputRef.current,
          NodeFilter.SHOW_TEXT,
          null
        )
        let node
        let targetNode = null
        let targetOffset = 0

        while ((node = treeWalker.nextNode())) {
          const nodeLength = node.textContent?.length || 0
          if (currentOffset + nodeLength >= textOffset) {
            targetNode = node
            targetOffset = textOffset - currentOffset
            break
          }
          currentOffset += nodeLength
        }

        if (targetNode) {
          selection.collapse(targetNode, Math.min(targetOffset, targetNode.textContent?.length || 0))
        }
      } catch (error) {
        // If cursor restoration fails, focus at the end
        selection.collapse(inputRef.current, inputRef.current.childNodes.length)
      }
    }
  }, [])

  // Update highlights when value changes
  useEffect(() => {
    highlightAtSymbols()
  }, [value, highlightAtSymbols])

  // Handle placeholder for contentEditable
  useEffect(() => {
    if (!inputRef.current) return

    const placeholder = getPlaceholder()
    const shouldShowPlaceholder = !value.trim() && !isFocused

    if (shouldShowPlaceholder && !inputRef.current.hasAttribute('data-placeholder-visible')) {
      inputRef.current.setAttribute('data-placeholder-visible', 'true')
      inputRef.current.style.color = 'rgba(168, 85, 247, 0.7)' // purple-200/70
      inputRef.current.style.fontWeight = '300' // font-light
    } else if (!shouldShowPlaceholder && inputRef.current.hasAttribute('data-placeholder-visible')) {
      inputRef.current.removeAttribute('data-placeholder-visible')
      inputRef.current.style.color = 'white'
      inputRef.current.style.fontWeight = 'normal'
    }
  }, [value, isFocused])
  
  return (
    <div className="relative">
      <div
        className={`
          relative flex items-center gap-3
          bg-black/20 border-2 rounded-2xl cursor-text
          transition-all duration-300 ease-out
          ${isFocused
            ? 'border-purple-400'
            : 'border-purple-400/50 hover:border-purple-400'
          }
          px-6 py-4
        `}
        onClick={() => {
          inputRef.current?.focus()
        }}
      >
        {/* Glow effect behind input */}
        <div className={`
          absolute inset-0 rounded-2xl transition-opacity duration-300
          ${isFocused ? 'opacity-100' : 'opacity-0'}
          bg-gradient-to-r from-purple-500/10 via-pink-500/5 to-indigo-500/10
          blur-sm -z-10
        `} />

        {/* Input Field with @ highlighting */}
        <div
          ref={inputRef as any}
          contentEditable
          suppressContentEditableWarning
          onInput={(e) => {
            const newValue = e.currentTarget.textContent || ''
            setValue(newValue)
          }}
          onFocus={() => setIsFocused(true)}
          onBlur={() => {
            // Delay hiding focus to allow suggestion clicks
            setTimeout(() => setIsFocused(false), 150)
          }}
          onKeyDown={handleKeyDown}
          className="
            flex-1 relative z-10 min-h-[1.5rem]
            bg-transparent border-none outline-none
            text-lg text-white
            font-geist-mono tracking-wide
          "
          aria-label="Command input"
          spellCheck={false}
          style={{
            whiteSpace: 'pre-wrap',
            wordBreak: 'break-word'
          }}
        />

        {/* Subtle inner glow */}
        {/* <div className={`
          absolute inset-[1px] rounded-2xl transition-opacity duration-300 pointer-events-none
          ${isFocused ? 'opacity-30' : 'opacity-0'}
          bg-gradient-to-r from-purple-400/20 to-pink-400/20
        `} /> */}
      </div>

      {/* AI Provider Dropdown */}
      {isAIDropdownOpen && value.endsWith('@') && (
        <div className="
          absolute top-full left-0 right-0 mt-2 z-50
          bg-black/80 backdrop-blur-md border border-purple-300/20 rounded-2xl
          shadow-2xl shadow-purple-500/20
        ">
          {AI_PROVIDERS.map((provider, index) => (
            <div
              key={provider.id}
              className={`
                px-6 py-3 cursor-pointer transition-colors duration-200
                ${index === selectedAIProviderIndex
                  ? 'bg-purple-500/20 text-purple-200'
                  : 'text-white hover:bg-purple-500/10 hover:text-purple-100'
                }
                ${index === 0 ? 'rounded-t-2xl' : ''}
                ${index === AI_PROVIDERS.length - 1 ? 'rounded-b-2xl' : ''}
              `}
              onClick={() => selectAIProvider(provider)}
              onMouseEnter={() => setSelectedAIProviderIndex(index)}
              onMouseLeave={() => setSelectedAIProviderIndex(-1)}
            >
              <div className="flex items-center gap-3">
                <span className={`text-lg ${provider.color}`}><img src={provider.icon} alt={provider.name} className="w-4 h-4" /></span>
                <span className="font-geist-mono text-sm">{provider.name}</span>
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Suggestions Dropdown */}
      {suggestions.length > 0 && (isFocused || selectedSuggestionIndex >= 0) && !isAIDropdownOpen && (
        <div className="
          absolute top-full left-0 right-0 mt-2 z-50
          bg-black/80 backdrop-blur-md border border-purple-300/20 rounded-2xl
          shadow-2xl shadow-purple-500/20 max-h-80 overflow-y-auto
          scrollbar-hide
        ">
          {suggestions.map((suggestion, index) => (
            <div
              key={index}
              className={`
                px-6 py-3 cursor-pointer transition-colors duration-200
                ${index === selectedSuggestionIndex
                  ? 'bg-purple-500/20 text-purple-200'
                  : 'text-white hover:bg-purple-500/10 hover:text-purple-100'
                }
                ${index === 0 ? 'rounded-t-2xl' : ''}
                ${index === suggestions.length - 1 ? 'rounded-b-2xl' : ''}
              `}
              onClick={() => selectSuggestion(suggestion)}
              onMouseEnter={() => setSelectedSuggestionIndex(index)}
              onMouseLeave={() => setSelectedSuggestionIndex(-1)}
            >
              <div className="flex items-center gap-3">
                {/* <span className="text-purple-400/70 text-sm">#{index + 1}</span> */}
                <span className="font-geist-mono text-sm">{suggestion}</span>
              </div>
            </div>
          ))}

          {/* Loading indicator for suggestions */}
          {isLoadingSuggestions && (
            <div className="px-6 py-3 text-purple-400/70 text-sm font-geist-mono">
              <div className="flex items-center gap-2">
                <div className="w-2 h-2 bg-purple-400 rounded-full animate-pulse"></div>
                <span>Loading suggestions...</span>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  )
}