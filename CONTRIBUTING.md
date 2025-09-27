# Contributing to BrowserOS

Thank you for your interest in contributing to BrowserOS! We welcome contributions from developers of all skill levels. This guide will help you get started with contributing to our AI-powered browser built on Chromium.

## 📋 Table of Contents

- [Code of Conduct](#code-of-conduct)
- [Getting Started](#getting-started)
- [Project Architecture](#project-architecture)
- [Development Setup](#development-setup)
- [Development Workflow](#development-workflow)
- [Contribution Areas](#contribution-areas)
- [Coding Standards](#coding-standards)
- [Testing](#testing)
- [Pull Request Process](#pull-request-process)
- [Issue Reporting](#issue-reporting)
- [Community](#community)

## 🤝 Code of Conduct

By participating in this project, you agree to abide by our community standards:

- Be respectful and inclusive
- Focus on what's best for the community
- Show empathy towards other community members
- Accept constructive criticism gracefully

## 🚀 Getting Started

### Prerequisites

Before you begin, ensure you have the following installed:

- **Node.js** (v18 or higher) - for the BrowserOS extension development
- **Python 3** - for the Chromium build system
- **Git** - version control
- **Chrome/Chromium** browser for testing
- **Code editor** (VS Code recommended)

For **Chromium browser development** (optional, only if building the full browser):
- **~100GB of free disk space** (for Chromium source)
- **~8GB RAM minimum** (16GB+ recommended)
- **Xcode and Command Line Tools** (macOS)
- **Visual Studio Build Tools** (Windows)
- **Build essentials** (Linux)

### Fork and Clone

1. Fork the BrowserOS repository on GitHub
2. Clone your forked repository:
   ```bash
   git clone https://github.com/YOUR_USERNAME/BrowserOS.git
   cd BrowserOS
   ```

3. Add the upstream remote:
   ```bash
   git remote add upstream https://github.com/browseros-ai/BrowserOS.git
   ```

## 🏗️ Project Architecture

BrowserOS consists of two main components:

### 1. BrowserOS Extension (`agent/`)
The main AI agent that runs as a BrowserOS extension, providing:
- **AI-powered browser automation** using LLM agents
- **Multi-tab management** and browser context handling
- **Tool system** for extensible browser operations
- **MCP (Model Context Protocol) integration** for external services
- **React-based UI** with side panel and new tab interfaces

**Key Technologies:**
- TypeScript, React, Tailwind CSS
- LangChain for LLM integration
- Puppeteer-core for browser automation
- Zod for schema validation
- Vitest for testing
- Webpack for bundling

### 2. Chromium Browser (`build/`, `chromium_patches/`)
A custom Chromium build with AI-native features:
- **Python build system** for orchestrating Chromium compilation
- **Patch system** for customizing Chromium behavior
- **Multi-platform support** (macOS, Windows, Linux)
- **Automated signing and packaging**

**Key Technologies:**
- Python build orchestration
- GN/Ninja build system (Chromium's build tools)
- Platform-specific packaging (DMG, MSI, AppImage)

## 🛠️ Development Setup

### BrowserOS Extension Development (Recommended for Most Contributors)

Most contributors will work on the BrowserOS extension in the `agent/` directory:

1. **Navigate to the agent directory:**
   ```bash
   cd agent
   ```

2. **Install dependencies:**
   ```bash
   npm install
   ```

3. **Create environment file:**
   ```bash
   # Create .env file with your API keys
   echo "LITELLM_API_KEY=your_api_key_here" > .env
   ```

4. **Build the extension:**
   ```bash
   # Development build with file watching
   npm run build:watch
   
   # Or one-time development build
   npm run build:dev
   ```

5. **Load extension in BrowserOS:**
   - Open BrowserOS and navigate to `chrome://extensions/`
   - Enable "Developer mode" (top-right toggle)
   - Click "Load unpacked" and select the `agent/dist` directory
   - The extension will appear with a side panel (Cmd/Ctrl+E to toggle)

### Environment Variables

Create a `.env` file in the `agent/` directory:

```env
# Required for LLM provider access
LITELLM_API_KEY=your_api_key_here

# Optional: For specific providers
OPENAI_API_KEY=your_openai_key
ANTHROPIC_API_KEY=your_anthropic_key
```

### VS Code Setup

Recommended VS Code extensions:
- **TypeScript and JavaScript Language Features**
- **ESLint** - for code linting
- **Prettier** - for code formatting  
- **Tailwind CSS IntelliSense** - for CSS classes
- **Vitest** - for test runner integration

**Debugging:**
- Launch configurations are available in `.vscode/launch.json`
- Use "Extension + Dev Server" configuration for full debugging
- Set breakpoints in TypeScript source files

### Chromium Browser Development (Advanced)

Only needed if you're modifying browser-level features:

1. **Setup Chromium source** (requires ~100GB disk space):
   ```bash
   # Follow Chromium's official guide to checkout source
   # This typically goes in a separate directory outside this repo
   ```

2. **Configure and build:**
   ```bash
   # Debug build (for development)
   python build/build.py --chromium-src /path/to/chromium/src --config build/config/debug.yaml

   # Release build (for production)
   # macOS:
   python build/build.py --chromium-src /path/to/chromium/src --config build/config/release.macos.yaml
   # Linux:
   python build/build.py --chromium-src /path/to/chromium/src --config build/config/release.linux.yaml
   # Windows:
   python build/build.py --chromium-src /path/to/chromium/src --config build/config/release.windows.yaml
   ```

3. **Alternative: Apply patches and build manually:**
   ```bash
   # If you prefer to run steps individually
   python build/build.py --chromium-src /path/to/chromium/src --apply-patches --build
   ```

For detailed Chromium build instructions, see [docs/BUILD.md](docs/BUILD.md).

Before contributing, let's understand how you can help:

## 🌟 Contribution Areas

### 1. 🤖 AI Agent & Tools Development
**Skill Level:** Intermediate to Advanced | **Impact:** High

The heart of BrowserOS is its AI agent system. You can contribute by:

- **Creating new tools** for browser automation (see `agent/src/lib/tools/`)
- **Improving agent planning and execution** logic
- **Adding MCP (Model Context Protocol) integrations** for external services
- **Enhancing LLM provider support** (Claude, OpenAI, Ollama, local models)

**Example tool areas needed:**
- E-commerce automation (shopping, price comparison)
- Social media management
- Data extraction and analysis
- Form filling and submission
- Calendar and email integration

### 2. 🎨 UI/UX Development  
**Skill Level:** Beginner to Intermediate | **Impact:** High

Improve the user interface and experience:

- **React components** in the side panel (`agent/src/sidepanel/`)
- **New tab page** enhancements (`agent/src/newtab/`)
- **Tailwind CSS styling** and responsive design
- **Accessibility improvements** (a11y)
- **User onboarding flows**

### 3. 🧪 Testing & Quality Assurance
**Skill Level:** Beginner to Intermediate | **Impact:** High

Help ensure BrowserOS is reliable:

- **Unit tests** with Vitest for individual components
- **Integration tests** for agent workflows
- **End-to-end testing** of browser automation
- **Performance testing** and optimization
- **Bug reproduction** and test case creation

### 4. 📚 Documentation & Examples
**Skill Level:** Beginner | **Impact:** Medium

Make BrowserOS more accessible:

- **User guides** and tutorials
- **API documentation** for tool developers  
- **Code examples** and demos
- **Video tutorials** and screencasts
- **Translation** of documentation

### 5. 🔧 Browser Engine Development
**Skill Level:** Advanced | **Impact:** Medium

For experienced systems programmers:

- **Chromium patches** for AI-native features
- **Build system improvements** (`build/` directory)
- **Cross-platform packaging** and distribution
- **Performance optimizations**
- **Security enhancements**

### 6. 🌐 Infrastructure & DevOps
**Skill Level:** Intermediate to Advanced | **Impact:** Medium

Support the development process:

- **CI/CD pipeline** improvements
- **Automated testing** infrastructure
- **Release automation**
- **Development tools** and scripts
- **Monitoring and analytics**

## 🔄 Development Workflow

### 1. Choose Your Contribution
- Browse [GitHub Issues](https://github.com/browseros-ai/BrowserOS/issues) or [Good First Issues](https://github.com/browseros-ai/BrowserOS/labels/good%20first%20issue)
- Join our [Discord](https://discord.gg/YKwjt5vuKr) to discuss ideas
- Check the [contribution areas](#contribution-areas) above

### 2. Create a Branch
```bash
git checkout -b feature/your-feature-name
# or
git checkout -b fix/bug-description
```

### 3. Make Changes
- Follow our [coding standards](#coding-standards) 
- Write tests for new functionality
- Update documentation as needed
- Test your changes thoroughly

### 4. Run Quality Checks

**For BrowserOS Extension development:**
```bash
cd agent

# Lint your code
npm run lint
npm run lint:fix  # Auto-fix issues

# Run tests
npm run test                    # Watch mode
npm run test:run               # Single run
npm run test:coverage          # With coverage
npm test -- path/to/file.test.ts  # Specific test

# Build and test the extension
npm run build:dev
# Load in BrowserOS and test manually
```

**For Chromium development:**
```bash
# Test your patches
python build/build.py --chromium-src /path/to/chromium/src --apply-patches --build

# Run Chromium tests (if applicable)
# This varies based on what you're changing
```

### 5. Commit Changes

We use [Conventional Commits](https://www.conventionalcommits.org/):

```bash
git commit -m "feat: add new e-commerce automation tool"
git commit -m "fix: resolve tab switching race condition"
git commit -m "docs: add tool development guide"
git commit -m "test: add unit tests for BrowserAgent planning"
git commit -m "refactor: simplify tool registration logic"
```

**Commit Types:**
- `feat`: New features
- `fix`: Bug fixes
- `docs`: Documentation changes
- `style`: Code style changes (formatting, etc.)
- `refactor`: Code refactoring
- `test`: Adding or updating tests
- `chore`: Build process, auxiliary tools, etc.

### 6. Push and Create PR
```bash
git push origin your-branch-name
```

Then create a Pull Request on GitHub with:
- **Clear title** using conventional commit format
- **Detailed description** explaining what and why
- **Screenshots/videos** for UI changes
- **Links to related issues** using `Fixes #123`

## 📝 Coding Standards

Our codebase follows strict standards to ensure consistency and maintainability.

### TypeScript Guidelines

- **Strict typing**: Always declare types for variables and functions
- **No `any`**: Avoid using `any` type; create proper types instead  
- **Zod schemas**: Use Zod schemas instead of plain TypeScript interfaces
- **Path aliases**: Use `@/lib` instead of relative imports like `../`
- **Import organization**: Group imports (external → internal → types)

### Code Style (Standard.js + Extensions)

- **2-space indentation**
- **Single quotes** (except to avoid escaping)  
- **No semicolons** (unless required for disambiguation)
- **Space after keywords**: `if (condition)`
- **Always use `===` and `!==`**
- **Trailing commas** in multiline structures

### Naming Conventions

- **Classes**: `PascalCase` (e.g., `BrowserAgent`, `ToolManager`)
- **Variables/Functions**: `camelCase` (e.g., `getUserData`, `executeTask`)
- **Files**: 
  - Classes exporting same-named class: `PascalCase` (e.g., `BrowserContext.ts`)
  - Utilities/functions: `lowercase` (e.g., `profiler.ts`, `types.ts`)
  - React components: `PascalCase` (e.g., `UserProfile.tsx`)
- **Directories**: `kebab-case` (e.g., `auth-wizard`, `tab-operations`)
- **Constants**: `UPPERCASE` (e.g., `MAX_ITERATIONS`, `DEFAULT_TIMEOUT`)
- **Private methods**: Prefix with `_` (e.g., `_validateInput()`)

### Zod Schema Pattern (Required)

**Always use Zod schemas instead of TypeScript interfaces:**

```typescript
import { z } from 'zod'

// Define schema with inline comments
export const ToolInputSchema = z.object({
  action: z.enum(['click', 'type', 'navigate']),  // Action to perform
  target: z.string().min(1),  // Target element description
  value: z.string().optional(),  // Optional input value
  timeout: z.number().positive().default(5000)  // Timeout in milliseconds
})

// Infer TypeScript type
export type ToolInput = z.infer<typeof ToolInputSchema>
```

### Tool Development Guidelines

When creating new tools for the agent system:

```typescript
import { DynamicStructuredTool } from '@langchain/core/tools'
import { ExecutionContext } from '@/lib/runtime/ExecutionContext'

// 1. Define input schema with Zod
const MyToolInputSchema = z.object({
  // ... your schema
})

// 2. Create tool class
export class MyTool {
  constructor(private executionContext: ExecutionContext) {}
  
  async execute(input: MyToolInput): Promise<ToolOutput> {
    // Implementation
    return toolSuccess('Task completed')
  }
}

// 3. Create factory function for LangChain integration
export function createMyTool(executionContext: ExecutionContext): DynamicStructuredTool {
  const myTool = new MyTool(executionContext)
  
  return new DynamicStructuredTool({
    name: 'my_tool',
    description: 'Clear description of what this tool does',
    schema: MyToolInputSchema,
    func: async (args): Promise<string> => {
      const result = await myTool.execute(args)
      return JSON.stringify(result)
    }
  })
}
```

### Comments & Documentation

- **JSDoc for public APIs**: Document classes, methods, and complex functions
- **Inline comments**: Use `// comment` for logic explanation
- **Avoid obvious comments**: Let code be self-documenting
- **Explain why, not what**: Focus on reasoning behind decisions
- **Two spaces before inline comments**: `const x = 5  // Timeout in seconds`

### Function Guidelines

- **Keep functions short**: <20 lines ideally
- **Single responsibility**: Each function should do one thing
- **Early returns**: Use guard clauses to reduce nesting
- **Descriptive names**: Use verbs (`getUserData`, `validateInput`)
- **RO-RO pattern**: Receive Object, Return Object for complex parameters

## 🧪 Testing

### Testing Framework

We use **Vitest** for all testing (never Jest):

```bash
cd agent

# Development testing
npm run test                    # Watch mode for development
npm run test:run               # Single run
npm run test:coverage          # Generate coverage report
npm run test:ui                # Interactive test UI

# Run specific tests
npm test -- path/to/file.test.ts
npm test -- --grep "should handle errors"

# Integration tests (requires API key)
LITELLM_API_KEY=your-key npm test -- file.integration.test.ts
```

### Test File Structure

Each test file should contain both unit and integration tests:

```typescript
import { describe, it, expect, vi } from 'vitest'  // Always use vitest
import { MyComponent } from './MyComponent'

describe('MyComponent', () => {
  // UNIT TESTS (2-3 tests max)
  it('should create instance successfully', () => {
    const component = new MyComponent()
    expect(component).toBeDefined()
  })
  
  it('should handle happy path correctly', () => {
    const component = new MyComponent()
    const result = component.process('valid input')
    expect(result.success).toBe(true)
  })
  
  it('should handle errors gracefully', () => {
    const component = new MyComponent()
    expect(() => component.process('')).toThrow()
  })
  
  // INTEGRATION TEST (1 test, requires API key)
  it('should integrate with real services', async () => {
    if (!process.env.LITELLM_API_KEY) {
      console.log('Skipping integration test - no API key')
      return
    }
    
    const component = new MyComponent()
    const result = await component.processWithLLM('test input')
    expect(result).toBeDefined()
  }, 10000)  // 10s timeout for integration tests
})
```

### Testing Philosophy

**Test behavior, not implementation:**

1. **Test the contract**: What the code promises to do
2. **Test edge cases**: Error handling and boundary conditions  
3. **Keep it simple**: 2-3 unit tests + 1 integration test max
4. **Access private methods freely**: Use `component._privateMethod()` for verification
5. **Test method calls and state changes**: Verify methods are called when expected

**What to test:**
- ✅ Public method behavior
- ✅ Error handling
- ✅ State changes
- ✅ Integration with real dependencies
- ❌ Mock return values (don't test mocks)
- ❌ Implementation details

### Test File Organization

Place test files next to source files:
```
agent/src/lib/
├── agent/
│   ├── BrowserAgent.ts
│   ├── BrowserAgent.test.ts     # Unit + integration tests
├── tools/
│   ├── navigation/
│   │   ├── NavigationTool.ts
│   │   └── NavigationTool.test.ts
```

## 📋 Pull Request Process

### Before Submitting

1. **Sync with upstream**:
   ```bash
   git fetch upstream
   git checkout main
   git merge upstream/main
   git checkout your-branch
   git rebase main
   ```

2. **Run all quality checks**:
   ```bash
   cd agent
   npm run lint          # Check code style
   npm run lint:fix      # Fix auto-fixable issues
   npm run test:run      # Run all tests
   npm run build:dev     # Ensure it builds
   ```

3. **Test manually**:
   - Load extension in BrowserOS and test your changes
   - Verify no regressions in existing functionality
   - Test edge cases and error scenarios

### PR Requirements

Your PR should include:

- **Clear title**: Use conventional commit format (e.g., "feat: add e-commerce automation tool")
- **Detailed description**: 
  - What changes were made
  - Why they were necessary  
  - How to test the changes
- **Link issues**: Reference related issues with `Fixes #123` or `Closes #456`
- **Screenshots/videos**: For UI changes, include before/after visuals
- **Tests**: Add unit tests for new functionality
- **Documentation**: Update relevant docs and comments

**PR Template:**
```markdown
## Description
Brief description of changes

## Type of Change
- [ ] Bug fix
- [ ] New feature  
- [ ] Breaking change
- [ ] Documentation update

## Testing
- [ ] Unit tests pass
- [ ] Manual testing completed
- [ ] Integration tests pass (if applicable)

## Screenshots
(Include for UI changes)

## Checklist
- [ ] Code follows style guidelines
- [ ] Self-review completed
- [ ] Tests added for new functionality
- [ ] Documentation updated
```

### CLA Signing (Required)

All contributors must sign our Contributor License Agreement:

1. The CLA bot will comment on your first PR
2. Review the [CLA document](CLA.md) 
3. Comment on your PR: `I have read the CLA Document and I hereby sign the CLA`
4. The bot will record your signature (one-time process)

### Review Process

1. **Automated checks**: 
   - Linting and formatting
   - Unit tests
   - Build verification
   - Security scans

2. **Code review**:
   - At least one maintainer approval required
   - Focus on code quality, security, and maintainability
   - May request changes or improvements

3. **Manual testing**:
   - For significant changes, maintainers will test manually
   - Complex features may require additional testing

4. **Merge**:
   - Squash and merge after approval
   - Your contribution will be included in the next release

## 🐛 Issue Reporting

### Bug Reports

Use our [bug report template](https://github.com/browseros-ai/BrowserOS/issues/new?template=bug_report.md):

**Required Information:**
- **Clear title**: Describe the issue concisely
- **Environment**: OS, browser version, BrowserOS version
- **Steps to reproduce**: Detailed, numbered steps
- **Expected behavior**: What should happen
- **Actual behavior**: What actually happens  
- **Screenshots/videos**: Visual evidence helps immensely
- **Console logs**: Check browser dev tools for errors
- **Extension logs**: Check the extension's background page console

**Example:**
```markdown
**Bug**: Agent fails to click buttons on dynamically loaded content

**Environment**: 
- OS: macOS 14.1
- Browser: Chrome 120.0.6099.109  
- BrowserOS: v0.1.0

**Steps to reproduce:**
1. Navigate to example.com
2. Click "Load More" button
3. Try to click newly loaded button
4. Agent reports "Element not found"

**Expected**: Agent should wait for content to load and click button
**Actual**: Agent immediately fails without waiting

**Console logs**: 
```
Error: Element not found: button[data-id="new-item"]
```

### Feature Requests

Use our [feature request template](https://github.com/browseros-ai/BrowserOS/issues/new?template=feature_request.md):

- **Problem description**: What problem does this solve?
- **Proposed solution**: How should it work?
- **User stories**: "As a user, I want..."
- **Use cases**: Real-world scenarios
- **Alternatives considered**: Other approaches you've thought of
- **Implementation ideas**: Technical suggestions (optional)

### Issue Labels

We use these labels to organize issues:

**Type:**
- `bug` - Something isn't working
- `enhancement` - New feature or improvement
- `documentation` - Documentation improvements
- `question` - Need clarification or help

**Priority:**
- `critical` - Breaks core functionality
- `high` - Important improvement
- `medium` - Nice to have
- `low` - Minor improvement

**Difficulty:**
- `good first issue` - Perfect for newcomers
- `help wanted` - Community contributions welcome
- `advanced` - Requires deep knowledge

**Area:**
- `agent` - AI agent system
- `ui` - User interface
- `browser` - Chromium integration
- `tools` - Agent tools
- `testing` - Test-related
- `build` - Build system

## 💬 Community

### Communication Channels

- **[Discord](https://discord.gg/YKwjt5vuKr)** 🔥 - Real-time chat, get help, discuss ideas
  - `#general` - General discussion
  - `#development` - Technical discussions  
  - `#help` - Get assistance
  - `#showcase` - Show off your contributions
- **[GitHub Issues](https://github.com/browseros-ai/BrowserOS/issues)** - Bug reports and feature requests
- **[GitHub Discussions](https://github.com/browseros-ai/BrowserOS/discussions)** - Long-form discussions
- **[Twitter/X](https://twitter.com/browseros_ai)** - Updates and announcements

### Getting Help

1. **Search first**: Check existing issues and discussions
2. **Discord**: Join for real-time help from the community
3. **Documentation**: Check `docs/` and `agent/docs/` folders
4. **Code comments**: Many functions have helpful inline docs
5. **Create an issue**: If you can't find an answer, create a detailed issue

### Recognition

We value all contributors! Recognition includes:

- **Release notes**: Major contributions are highlighted
- **Contributors list**: Added to README and project pages
- **Discord roles**: Special contributor roles and badges
- **Shout-outs**: Recognition on social media
- **Maintainer path**: Outstanding contributors may be invited as maintainers

## 🎯 Good First Issues

New to the project? Start with these:

**🟢 Beginner (Good First Issues):**
- Documentation improvements and typos
- Adding unit tests to existing code
- Small UI improvements and polish
- Code cleanup and refactoring
- Adding inline comments and JSDoc

**🟡 Intermediate:**
- New React components for the UI
- Bug fixes in the agent system
- Performance optimizations
- Adding new simple tools

**🔴 Advanced:**
- Complex agent features
- Chromium browser integration
- Build system improvements
- Architecture changes

Browse [Good First Issues](https://github.com/browseros-ai/BrowserOS/labels/good%20first%20issue) to get started!

## 📚 Learning Resources

### Essential Technologies

- **[TypeScript Handbook](https://www.typescriptlang.org/docs/)** - Our primary language
- **[React Documentation](https://react.dev/)** - UI framework
- **[Chrome Extension Docs](https://developer.chrome.com/docs/extensions/)** - Extension APIs
- **[Zod Documentation](https://zod.dev/)** - Schema validation (required)
- **[Vitest Guide](https://vitest.dev/guide/)** - Testing framework
- **[LangChain](https://js.langchain.com/docs/)** - LLM integration
- **[Tailwind CSS](https://tailwindcss.com/)** - Styling

### BrowserOS-Specific Docs

- **[Architecture Overview](agent/docs/agent-design-new.md)** - System design
- **[Build Process](docs/BUILD.md)** - Chromium build guide
- **[Tool Development](agent/docs/tools-design.md)** - Creating agent tools
- **[API Reference](agent/docs/)** - Technical documentation

### Video Tutorials

- [Setting up Development Environment](https://www.youtube.com/watch?v=example)
- [Creating Your First Tool](https://www.youtube.com/watch?v=example)
- [Understanding the Agent System](https://www.youtube.com/watch?v=example)

## ❓ FAQ

**Q: How do I set up the development environment?**
A: Follow the [BrowserOS Extension Development](#BrowserOS-extension-development-recommended-for-most-contributors) section above. Most contributors only need the extension setup.

**Q: I'm new to open source. Where should I start?**
A: Welcome! Start with [Good First Issues](https://github.com/browseros-ai/BrowserOS/labels/good%20first%20issue), join our Discord, and don't hesitate to ask questions.

**Q: How do I run the extension locally?**
A: Run `npm run build:dev` in the `agent/` directory, then load the `dist` folder as an unpacked extension in BrowserOS.

**Q: Do I need to build the full Chromium browser?**
A: No! Most contributors only work on the BrowserOS extension. Chromium building is only needed for browser-level features.

**Q: How long does code review take?**
A: We aim for 2-3 business days for most PRs. Complex changes may take longer.

**Q: Can I work on multiple issues simultaneously?**
A: We recommend focusing on one issue at a time, especially when starting out.

**Q: Do I need an API key to contribute?**
A: For basic development, no. You'll need one for testing LLM features, but you can contribute UI improvements, documentation, and tests without it.

**Q: The codebase seems large. How do I navigate it?**
A: Start with the `agent/src/` directory. The main entry points are `lib/core/NxtScape.ts` and `lib/agent/BrowserAgent.ts`. Join Discord for guidance!

---

## 🙏 Thank You

Thank you for your interest in contributing to BrowserOS! We're building the future of AI-powered browsing, and every contribution—whether it's a bug fix, new feature, documentation improvement, or even just feedback—helps make that vision a reality.

Together, we're creating tools that will transform how people interact with the web. Welcome to the team! 🚀

**Ready to contribute?** Join our [Discord](https://discord.gg/YKwjt5vuKr) and say hello! 👋
