import { z } from 'zod'
import { DynamicStructuredTool } from '@langchain/core/tools'
import { ExecutionContext } from '@/lib/runtime/ExecutionContext'
import { toolSuccess, toolError, type ToolOutput } from '@/lib/tools/tool.interface'

// Input schema for Nemo info tool
const NemoInfoToolInputSchema = z.object({
  topic: z.enum([
    'overview',
    'installation',
    'configuration',
    'automation',
    'api',
    'troubleshooting',
    'examples',
    'features'
  ]).describe('Information topic to retrieve about Nemo'),

  section: z.string()
    .optional()
    .describe('Specific section within the topic (optional)')
})

export type NemoInfoToolInput = z.infer<typeof NemoInfoToolInputSchema>

/**
 * NemoInfoTool - Provides comprehensive information about Nemo features and usage
 * Contains various README-style documentation for different aspects of Nemo
 */
export class NemoInfoTool {
  private executionContext: ExecutionContext

  constructor(executionContext: ExecutionContext) {
    this.executionContext = executionContext
  }

  async execute(input: NemoInfoToolInput): Promise<ToolOutput> {
    try {
      const { topic, section } = input

      const info = this._getTopicInfo(topic, section)

      if (!info) {
        return toolError(`No information found for topic: ${topic}${section ? `, section: ${section}` : ''}`)
      }

      return toolSuccess(JSON.stringify({
        topic,
        section: section || 'all',
        content: info
      }))
    } catch (error) {
      return toolError(`Failed to retrieve Nemo info: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  private _getTopicInfo(topic: string, section?: string): string | null {
    const infoDatabase = {
      overview: `# Nemo Overview

Nemo is an AI-powered browser automation platform that enables intelligent web interaction through natural language commands.

## Key Features
- **AI-Driven Automation**: Uses LLM providers (Claude, OpenAI, Ollama) to understand and execute browser tasks
- **Multi-Tab Support**: Seamlessly work across multiple browser tabs
- **Visual Element Recognition**: Can identify and interact with page elements using visual descriptions
- **Real-Time Streaming**: See AI thinking and actions in real-time
- **Extensible Tool System**: Modular architecture with specialized tools for different tasks

## Architecture
- **BrowserAgent**: Main unified agent handling task execution
- **Tool System**: Modular tools for navigation, interaction, planning, and validation
- **Browser Integration**: Direct Chrome extension APIs for tab management
- **LLM Integration**: Multi-provider support with streaming capabilities

## Use Cases
- Web scraping and data extraction
- Form automation and testing
- Cross-platform web workflows
- Browser-based task automation
- AI-assisted web navigation`,

      installation: `# Nemoallation Guide

## Prerequisites
- Chrome browser (latest version recommended)
- Node.js 18+ for development
- NPM or Yarn package manager

## Installation Steps

### 1. Chrome Extension Installation
\`\`\`bash
# Clone the repository
git clone https://github.com/nemo-ai/Nemo-agent
cd Nemo-agent

# Install dependencies
npm install

# Build the extension
npm run build
\`\`\`

### 2. Load Extension in Chrome
1. Open Chrome and navigate to \`chrome://extensions/\`
2. Enable "Developer mode" in the top right
3. Click "Load unpacked" and select the \`dist\` folder
4. The Nemo extension should now appear in your extensions

### 3. Configuration
1. Click the Nemo extension icon
2. Configure your LLM provider (Claude, OpenAI, or Ollama)
3. Set up API keys in the settings panel
4. Test the connection with a simple task

### 4. Development Setup
\`\`\`bash
# Development build with watch mode
npm run build:watch

# Run tests
npm test

# Lint code
npm run lint
\`\`\``,

      configuration: `# Nemo Configuration

## LLM Provider Setup

### Claude (Anthropic)
\`\`\`javascript
{
  "provider": "anthropic",
  "apiKey": "your-claude-api-key",
  "model": "claude-3-sonnet-20240229"
}
\`\`\`

### OpenAI
\`\`\`javascript
{
  "provider": "openai",
  "apiKey": "your-openai-api-key",
  "model": "gpt-4"
}
\`\`\`

### Ollama (Local)
\`\`\`javascript
{
  "provider": "ollama",
  "baseUrl": "http://localhost:11434",
  "model": "llama2"
}
\`\`\`

## Extension Settings
- **Auto-submit**: Automatically submit tasks when Enter is pressed
- **Glow Animation**: Visual feedback during browser interactions
- **Streaming**: Real-time display of AI thinking process
- **Tab Management**: Multi-tab operation preferences

## Environment Variables
\`\`\`bash
# For development
OPENAI_API_KEY=your-api-key
MOONDREAM_API_KEY=your-vision-api-key
\`\`\``,

      automation: `# Nemo Automation Capabilities

## Core Automation Features

### Navigation
- Navigate to URLs
- Handle page loading and redirects
- Manage browser history
- Multi-tab operations

### Element Interaction
- Click elements by description or coordinates
- Type text into input fields
- Clear form fields
- Scroll pages and elements
- Handle dropdowns and selections

### Data Extraction
- Extract structured data from pages
- Capture screenshots
- Get page text content
- Extract links and metadata
- Parse tables and lists

### Visual Recognition
- Click elements by visual description
- Type into fields using visual cues
- Screenshot analysis for element detection
- Computer vision integration

## Task Planning
- Automatic task classification (simple vs complex)
- Multi-step plan generation
- Iterative re-planning on failures
- Human-in-the-loop interventions

## Error Handling
- Automatic retry mechanisms
- Graceful failure recovery
- Loop detection and prevention
- User cancellation support`,

      api: `# Nemo API Reference

## Tool System

### Navigation Tools
- \`navigation_tool\`: Navigate to URLs
- \`interact_tool\`: Click and interact with elements
- \`scroll_tool\`: Scroll pages and elements
- \`search_tool\`: Search for text on pages
- \`refresh_browser_state_tool\`: Refresh page state

### Planning Tools
- \`classification_tool\`: Classify task complexity
- \`planner_tool\`: Generate multi-step plans
- \`todo_manager_tool\`: Manage task lists
- \`validator_tool\`: Validate task completion

### Utility Tools
- \`screenshot_tool\`: Capture page screenshots
- \`extract_tool\`: Extract structured data
- \`storage_tool\`: Browser storage operations
- \`date_tool\`: Date calculations
- \`human_input_tool\`: Request human intervention

### Tab Management
- \`tab_operations_tool\`: Create, close, switch tabs
- \`group_tabs_tool\`: Organize tabs into groups
- \`get_selected_tabs_tool\`: Get user-selected tabs

## JavaScript API
\`\`\`javascript
// Execute a task
await browserOS.execute("Navigate to Google and search for cats")

// Get browser state
const state = await browserOS.getBrowserState()

// Take screenshot
const screenshot = await browserOS.takeScreenshot()
\`\`\``,

      troubleshooting: `# Nemo Troubleshooting

## Common Issues

### Extension Not Loading
- Ensure Chrome Developer Mode is enabled
- Check that the \`dist\` folder is properly built
- Reload the extension in chrome://extensions/
- Check browser console for errors

### API Connection Issues
- Verify API keys are correctly configured
- Check network connectivity
- Ensure provider endpoints are accessible
- Test with curl or Postman first

### Task Execution Failures
- Check if the page has loaded completely
- Verify element selectors are correct
- Look for JavaScript errors in console
- Try refreshing the page state

### Performance Issues
- Limit concurrent tab operations
- Use simplified browser state when possible
- Clear browser cache and cookies
- Restart Chrome if memory usage is high

## Debug Mode
Enable debug logging:
\`\`\`javascript
localStorage.setItem('browserOS.debug', 'true')
\`\`\`

## Getting Help
- Check browser console for error messages
- Use the screenshot tool to verify page state
- Enable verbose logging in settings
- Report issues with detailed reproduction steps`,

      examples: `# Nemo Usage Examples

## Basic Navigation
\`\`\`
Navigate to https://example.com and click the login button
\`\`\`

## Form Automation
\`\`\`
Fill out the contact form with:
- Name: John Doe
- Email: john@example.com
- Message: Hello from Nemo
Then submit the form
\`\`\`

## Data Extraction
\`\`\`
Extract all product names and prices from this e-commerce page into a structured format
\`\`\`

## Multi-Tab Workflow
\`\`\`
Open 3 tabs:
1. Google.com - search for "weather"
2. YouTube.com - search for "tutorials"
3. GitHub.com - find trending repositories
Then take screenshots of all tabs
\`\`\`

## Complex Automation
\`\`\`
1. Login to the admin panel
2. Navigate to user management
3. Export all user data to CSV
4. Download the file
5. Verify the download completed
\`\`\`

## Visual Element Interaction
\`\`\`
Click the blue "Subscribe" button in the top right corner
Type "test@example.com" into the email signup field
\`\`\`

## Error Recovery
\`\`\`
If you encounter a CAPTCHA or need human verification,
pause and wait for me to complete it manually
\`\`\``,

      features: `# Nemo Features

## AI-Powered Automation
- **Natural Language Processing**: Understands complex instructions in plain English
- **Intelligent Planning**: Breaks down complex tasks into manageable steps
- **Context Awareness**: Maintains conversation history and task context
- **Adaptive Execution**: Adjusts strategy based on page content and errors

## Browser Integration
- **Multi-Tab Support**: Seamlessly work across multiple browser tabs
- **Real-Time State**: Live browser state monitoring and updates
- **Tab Management**: Create, organize, and manage browser tabs
- **Session Persistence**: Maintain state across browser sessions

## Visual Recognition
- **Element Detection**: Find elements using visual descriptions
- **Screenshot Analysis**: Computer vision for element identification
- **Coordinate-Based Actions**: Precise clicking and typing by coordinates
- **Visual Feedback**: Glow animations and crosshairs for user feedback

## Extensibility
- **Modular Tool System**: Easily add new tools and capabilities
- **Provider Flexibility**: Support for multiple LLM providers
- **Custom Agents**: Create specialized agents for specific workflows
- **API Integration**: Connect with external services and APIs

## User Experience
- **Streaming Interface**: Real-time display of AI thinking process
- **Human-in-the-Loop**: Request human intervention when needed
- **Error Recovery**: Graceful handling of failures and edge cases
- **Debug Support**: Comprehensive logging and troubleshooting tools

## Performance & Reliability
- **Loop Detection**: Prevents infinite loops and repetitive actions
- **Resource Management**: Efficient memory and CPU usage
- **Abort Controls**: Cancel long-running tasks at any time
- **State Validation**: Verify task completion and success`
    }

    const topicContent = infoDatabase[topic as keyof typeof infoDatabase]

    if (!topicContent) {
      return null
    }

    // If a specific section is requested, try to extract it
    if (section) {
      const sectionRegex = new RegExp(`## ${section}[\\s\\S]*?(?=## |$)`, 'i')
      const sectionMatch = topicContent.match(sectionRegex)
      if (sectionMatch) {
        return sectionMatch[0]
      }
      // If section not found, return full content with note
      return `${topicContent}\n\n*Note: Specific section "${section}" not found. Showing full ${topic} information.*`
    }

    return topicContent
  }
}

/**
 * Factory function to create NemoInfoTool for LangChain integration
 */
export function createNemoInfoTool(executionContext: ExecutionContext): DynamicStructuredTool {
  const browserOSInfoTool = new NemoInfoTool(executionContext)

  const dynamicTool = new DynamicStructuredTool({
    name: "nemo_info_tool",
    description: `Get comprehensive information about Nemoures, configuration, and usage.

    Available topics:
    - overview: General overview and key features
    - installation: Installation and setup guide
    - configuration: LLM provider and extension settings
    - automation: Automation capabilities and features
    - api: Tool system and JavaScript API reference
    - troubleshooting: Common issues and solutions
    - examples: Usage examples and workflows
    - features: Detailed feature descriptions

    Optionally specify a section within a topic for more focused information.`,

    schema: NemoInfoToolInputSchema,

    func: async (args): Promise<string> => {
      const result = await browserOSInfoTool.execute(args)
      return JSON.stringify(result)
    }
  })

  return dynamicTool
}