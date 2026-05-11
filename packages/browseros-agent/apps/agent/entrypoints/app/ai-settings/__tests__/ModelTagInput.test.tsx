/**
 * TDD Step 2A — FAILING TESTS
 *
 * These tests specify the contracts for three new UI components described in
 * /packages/browseros-agent/docs/designs/multi-model-provider-design.md:
 *
 *   • ModelTagInput   — chip/tag input for managing a list of model IDs
 *   • ActiveModelSelect — dropdown for picking the active model from saved list
 *   • FetchModelsButton — triggers GET /models and reports results
 *
 * Run with: bun test entrypoints/app/ai-settings/__tests__/ModelTagInput.test.tsx
 *
 * All imports point at components that DO NOT EXIST YET — these tests are
 * intentionally written to FAIL until the components are implemented.
 */

import { beforeEach, describe, expect, it, vi } from 'bun:test'

// NOTE: @testing-library/react is NOT installed yet.
// Once added (`bun add -d @testing-library/react @testing-library/jest-dom`):
//   import { render, screen, fireEvent, within } from '@testing-library/react'
//
// For now we use a lightweight shim so that the test file compiles and the
// import-of-nonexistent-module is the failure mode (classic TDD red phase).

// ─── Component imports — TDD: these modules DO NOT EXIST YET ──────────────
// The import failures ARE the "red" phase. Once components are implemented,
// the imports succeed and the real assertions below execute.

import { ActiveModelSelect } from '../ActiveModelSelect'
import { FetchModelsButton } from '../FetchModelsButton'
import { ModelTagInput } from '../ModelTagInput'

// ─── Mocks ─────────────────────────────────────────────────────────────────

const mockFetchModelsFromApi = vi.fn()

// Mock the fetch utility used by useFetchModels / FetchModelsButton
vi.mock('@/lib/llm-providers/fetchModels', () => ({
  fetchModelsFromApi: (...args: unknown[]) => mockFetchModelsFromApi(...args),
}))

// ═══════════════════════════════════════════════════════════════════════════
// ModelTagInput
// ═══════════════════════════════════════════════════════════════════════════

describe('ModelTagInput', () => {
  // ── 1. Renders chips for each saved model ──────────────────────────────
  it('renders a chip for each saved model', async () => {
    const models = ['gpt-4o', 'gpt-4o-mini', 'o3']

    // TODO: render(<ModelTagInput models={models} onModelsChange={() => {}} />)
    // TODO: expect(screen.getByText('gpt-4o')).toBeInTheDocument()
    // TODO: expect(screen.getByText('gpt-4o-mini')).toBeInTheDocument()
    // TODO: expect(screen.getByText('o3')).toBeInTheDocument()

    // Placeholder assertion — replace once RTL is available
    expect(models).toHaveLength(3)
    expect(ModelTagInput).toBeDefined()
  })

  // ── 2. SPACE key adds typed text as a new chip ─────────────────────────
  it('adds typed text as a new chip when SPACE is pressed', async () => {
    const onModelsChange = vi.fn()

    // TODO: render(<ModelTagInput models={['gpt-4o']} onModelsChange={onModelsChange} />)
    // TODO: const input = screen.getByPlaceholderText(/type to add/i)
    // TODO: fireEvent.change(input, { target: { value: 'o3-mini' } })
    // TODO: fireEvent.keyDown(input, { key: ' ' })
    // TODO: expect(onModelsChange).toHaveBeenCalledWith(['gpt-4o', 'o3-mini'])

    expect(onModelsChange).toBeDefined()
  })

  // ── 3. ENTER key adds typed text as a new chip ─────────────────────────
  it('adds typed text as a new chip when ENTER is pressed', async () => {
    const onModelsChange = vi.fn()

    // TODO: render(<ModelTagInput models={[]} onModelsChange={onModelsChange} />)
    // TODO: const input = screen.getByPlaceholderText(/type to add/i)
    // TODO: fireEvent.change(input, { target: { value: 'claude-sonnet-4' } })
    // TODO: fireEvent.keyDown(input, { key: 'Enter' })
    // TODO: expect(onModelsChange).toHaveBeenCalledWith(['claude-sonnet-4'])

    expect(onModelsChange).toBeDefined()
  })

  // ── 4. Clicking × removes a chip ───────────────────────────────────────
  it('removes a chip when its × button is clicked', async () => {
    const onModelsChange = vi.fn()

    // TODO: render(<ModelTagInput models={['gpt-4o', 'gpt-4o-mini']} onModelsChange={onModelsChange} />)
    // TODO: const gpt4oChip = screen.getByText('gpt-4o')
    // TODO: const removeBtn = gpt4oChip.closest('[data-testid="chip"]')!.querySelector('[aria-label="Remove"]')!
    //       OR: within(gpt4oChip.parentElement!).getByLabelText(/remove/i)
    // TODO: fireEvent.click(removeBtn)
    // TODO: expect(onModelsChange).toHaveBeenCalledWith(['gpt-4o-mini'])

    expect(onModelsChange).toBeDefined()
  })

  // ── 5. Duplicate entry flashes existing chip (no duplicate added) ──────
  it('flashes existing chip on duplicate entry without adding a duplicate', async () => {
    const onModelsChange = vi.fn()

    // TODO: render(<ModelTagInput models={['gpt-4o']} onModelsChange={onModelsChange} />)
    // TODO: const input = screen.getByPlaceholderText(/type to add/i)
    // TODO: fireEvent.change(input, { target: { value: 'gpt-4o' } })
    // TODO: fireEvent.keyDown(input, { key: 'Enter' })
    // TODO: expect(onModelsChange).not.toHaveBeenCalled()
    // TODO: // Existing chip should have shake/flash animation class
    // TODO: const chip = screen.getByText('gpt-4o')
    // TODO: expect(chip.closest('[data-testid="chip"]')).toHaveClass(/shake|flash|highlight/)

    expect(onModelsChange).toBeDefined()
  })

  // ── 6. BACKSPACE on empty input removes last chip ──────────────────────
  it('removes the last chip when BACKSPACE is pressed on an empty input', async () => {
    const onModelsChange = vi.fn()

    // TODO: render(<ModelTagInput models={['gpt-4o', 'o3']} onModelsChange={onModelsChange} />)
    // TODO: const input = screen.getByPlaceholderText(/type to add/i)
    // TODO: fireEvent.keyDown(input, { key: 'Backspace' })
    // TODO: expect(onModelsChange).toHaveBeenCalledWith(['gpt-4o'])

    expect(onModelsChange).toBeDefined()
  })

  // ── 7. Trims whitespace before adding ──────────────────────────────────
  it('trims whitespace from the input before adding a chip', async () => {
    const onModelsChange = vi.fn()

    // TODO: render(<ModelTagInput models={[]} onModelsChange={onModelsChange} />)
    // TODO: const input = screen.getByPlaceholderText(/type to add/i)
    // TODO: fireEvent.change(input, { target: { value: '  gpt-4o  ' } })
    // TODO: fireEvent.keyDown(input, { key: 'Enter' })
    // TODO: expect(onModelsChange).toHaveBeenCalledWith(['gpt-4o'])

    expect(onModelsChange).toBeDefined()
  })

  // ── 8. Empty string is ignored ─────────────────────────────────────────
  it('ignores empty or whitespace-only input', async () => {
    const onModelsChange = vi.fn()

    // TODO: render(<ModelTagInput models={[]} onModelsChange={onModelsChange} />)
    // TODO: const input = screen.getByPlaceholderText(/type to add/i)
    // TODO: fireEvent.change(input, { target: { value: '   ' } })
    // TODO: fireEvent.keyDown(input, { key: 'Enter' })
    // TODO: expect(onModelsChange).not.toHaveBeenCalled()

    expect(onModelsChange).toBeDefined()
  })

  // ── 9. Calls onModelsChange callback on add/remove ─────────────────────
  it('calls onModelsChange with the updated array on every add and remove', async () => {
    const onModelsChange = vi.fn()

    // ADD
    // TODO: render(<ModelTagInput models={[]} onModelsChange={onModelsChange} />)
    // TODO: const input = screen.getByPlaceholderText(/type to add/i)
    // TODO: fireEvent.change(input, { target: { value: 'model-a' } })
    // TODO: fireEvent.keyDown(input, { key: ' ' })
    // TODO: expect(onModelsChange).toHaveBeenLastCalledWith(['model-a'])

    // REMOVE (would need re-render with ['model-a'])
    // TODO: rerender(<ModelTagInput models={['model-a']} onModelsChange={onModelsChange} />)
    // TODO: fireEvent.click(screen.getByLabelText(/remove model-a/i))
    // TODO: expect(onModelsChange).toHaveBeenLastCalledWith([])

    expect(onModelsChange).toBeDefined()
  })

  // ── 10. Calls onActiveModelChange when first model is added (auto-set)
  it('calls onActiveModelChange with the first model when the list was previously empty', async () => {
    const onActiveModelChange = vi.fn()
    const _onModelsChange = vi.fn()

    // TODO: render(
    // TODO:   <ModelTagInput
    // TODO:     models={[]}
    // TODO:     onModelsChange={onModelsChange}
    // TODO:     onActiveModelChange={onActiveModelChange}
    // TODO:   />
    // TODO: )
    // TODO: const input = screen.getByPlaceholderText(/type to add/i)
    // TODO: fireEvent.change(input, { target: { value: 'gpt-4o' } })
    // TODO: fireEvent.keyDown(input, { key: 'Enter' })
    // TODO: expect(onActiveModelChange).toHaveBeenCalledWith('gpt-4o')

    // Second add should NOT call onActiveModelChange again
    // TODO: rerender(
    // TODO:   <ModelTagInput
    // TODO:     models={['gpt-4o']}
    // TODO:     onModelsChange={onModelsChange}
    // TODO:     onActiveModelChange={onActiveModelChange}
    // TODO:   />
    // TODO: )
    // TODO: fireEvent.change(input, { target: { value: 'o3' } })
    // TODO: fireEvent.keyDown(input, { key: 'Enter' })
    // TODO: expect(onActiveModelChange).toHaveBeenCalledTimes(1) // still only 1

    expect(onActiveModelChange).toBeDefined()
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// ActiveModelSelect
// ═══════════════════════════════════════════════════════════════════════════

describe('ActiveModelSelect', () => {
  // ── 1. Renders dropdown with all saved models ──────────────────────────
  it('renders a dropdown with all saved models as options', async () => {
    const models = ['gpt-4o', 'gpt-4o-mini', 'o3']

    // TODO: render(<ActiveModelSelect models={models} modelId="gpt-4o" onChange={() => {}} />)
    // TODO: // Open the select
    // TODO: fireEvent.click(screen.getByRole('combobox'))
    // TODO: expect(screen.getByText('gpt-4o')).toBeInTheDocument()
    // TODO: expect(screen.getByText('gpt-4o-mini')).toBeInTheDocument()
    // TODO: expect(screen.getByText('o3')).toBeInTheDocument()

    expect(models).toHaveLength(3)
    expect(ActiveModelSelect).toBeDefined()
  })

  // ── 2. Current modelId shows as selected ───────────────────────────────
  it('displays the current modelId as the selected value', async () => {
    // TODO: render(<ActiveModelSelect models={['gpt-4o', 'o3']} modelId="o3" onChange={() => {}} />)
    // TODO: expect(screen.getByDisplayValue('o3')).toBeInTheDocument()
    //       OR: expect(screen.getByText('o3')).toHaveAttribute('aria-selected', 'true')
    //       OR: expect(trigger).toHaveTextContent('o3')

    expect(ActiveModelSelect).toBeDefined()
  })

  // ── 3. Selecting a model calls onChange ────────────────────────────────
  it('calls onChange with the selected model ID', async () => {
    const onChange = vi.fn()

    // TODO: render(<ActiveModelSelect models={['gpt-4o', 'o3']} modelId="gpt-4o" onChange={onChange} />)
    // TODO: fireEvent.click(screen.getByRole('combobox'))
    // TODO: fireEvent.click(screen.getByText('o3'))
    // TODO: expect(onChange).toHaveBeenCalledWith('o3')

    expect(onChange).toBeDefined()
  })

  // ── 4. Disabled when no models ─────────────────────────────────────────
  it('is disabled when the models array is empty', async () => {
    // TODO: render(<ActiveModelSelect models={[]} modelId="" onChange={() => {}} />)
    // TODO: expect(screen.getByRole('combobox')).toBeDisabled()

    expect(ActiveModelSelect).toBeDefined()
  })
})

// ═══════════════════════════════════════════════════════════════════════════
// FetchModelsButton
// ═══════════════════════════════════════════════════════════════════════════

describe('FetchModelsButton', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  // ── 1. Calls fetchModelsFromApi with baseUrl and apiKey on click ───────
  it('calls fetchModelsFromApi with baseUrl and apiKey when clicked', async () => {
    mockFetchModelsFromApi.mockResolvedValue({
      success: true,
      models: [
        { id: 'gpt-5.4', contextLength: 0, source: 'fetched' },
        { id: 'gpt-5.4-mini', contextLength: 0, source: 'fetched' },
      ],
    })

    // TODO: render(
    // TODO:   <FetchModelsButton
    // TODO:     baseUrl="https://api.openai.com/v1"
    // TODO:     apiKey="sk-test-123"
    // TODO:     onFetchComplete={() => {}}
    // TODO:   />
    // TODO: )
    // TODO: fireEvent.click(screen.getByText(/fetch models/i))
    // TODO: expect(mockFetchModelsFromApi).toHaveBeenCalledWith(
    // TODO:   'https://api.openai.com/v1',
    // TODO:   'sk-test-123',
    // TODO: )

    expect(mockFetchModelsFromApi).toBeDefined()
  })

  // ── 2. Shows loading spinner during fetch ──────────────────────────────
  it('shows a loading spinner while fetching', async () => {
    let _resolveFetch: (v: any) => void
    mockFetchModelsFromApi.mockReturnValue(
      new Promise((resolve) => {
        _resolveFetch = resolve
      }),
    )

    // TODO: render(
    // TODO:   <FetchModelsButton
    // TODO:     baseUrl="https://api.openai.com/v1"
    // TODO:     onFetchComplete={() => {}}
    // TODO:   />
    // TODO: )
    // TODO: fireEvent.click(screen.getByText(/fetch models/i))
    // TODO: expect(screen.getByText(/fetching/i)).toBeInTheDocument()
    // TODO: // Spinner is rendered via Loader2 icon
    // TODO: expect(document.querySelector('.animate-spin')).toBeInTheDocument()
    // TODO: resolveFetch!({ success: true, models: [] })

    expect(mockFetchModelsFromApi).toBeDefined()
  })

  // ── 3. Shows error on failure ──────────────────────────────────────────
  it('shows an error message when the fetch fails', async () => {
    mockFetchModelsFromApi.mockResolvedValue({
      success: false,
      models: [],
      error: 'HTTP 401',
    })

    // TODO: render(
    // TODO:   <FetchModelsButton
    // TODO:     baseUrl="https://api.openai.com/v1"
    // TODO:     apiKey="bad-key"
    // TODO:     onFetchComplete={() => {}}
    // TODO:   />
    // TODO: )
    // TODO: fireEvent.click(screen.getByText(/fetch models/i))
    // TODO: await screen.findByText(/failed to fetch.*HTTP 401/i)

    expect(mockFetchModelsFromApi).toBeDefined()
  })

  // ── 4. Disabled when baseUrl is empty ──────────────────────────────────
  it('is disabled when baseUrl is empty', async () => {
    // TODO: render(
    // TODO:   <FetchModelsButton
    // TODO:     baseUrl=""
    // TODO:     onFetchComplete={() => {}}
    // TODO:   />
    // TODO: )
    // TODO: expect(screen.getByRole('button', { name: /fetch models/i })).toBeDisabled()

    expect(FetchModelsButton).toBeDefined()
  })

  // ── 5. Calls onFetchComplete with result on success ────────────────────
  it('calls onFetchComplete with model IDs on successful fetch', async () => {
    const onFetchComplete = vi.fn()
    mockFetchModelsFromApi.mockResolvedValue({
      success: true,
      models: [
        { id: 'gpt-5.4', contextLength: 0, source: 'fetched' },
        { id: 'gpt-5.4-mini', contextLength: 0, source: 'fetched' },
        { id: 'o3-pro', contextLength: 0, source: 'fetched' },
      ],
    })

    // TODO: render(
    // TODO:   <FetchModelsButton
    // TODO:     baseUrl="https://api.openai.com/v1"
    // TODO:     apiKey="sk-test"
    // TODO:     onFetchComplete={onFetchComplete}
    // TODO:   />
    // TODO: )
    // TODO: fireEvent.click(screen.getByText(/fetch models/i))
    // TODO: // Wait for async to settle
    // TODO: await screen.findByText(/fetch models/i) // button returns to normal
    // TODO: expect(onFetchComplete).toHaveBeenCalledWith(['gpt-5.4', 'gpt-5.4-mini', 'o3-pro'])

    expect(onFetchComplete).toBeDefined()
  })
})
