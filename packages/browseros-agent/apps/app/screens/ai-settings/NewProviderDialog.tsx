import { zodResolver } from '@hookform/resolvers/zod'
import Fuse from 'fuse.js'
import {
  Check,
  CheckCircle2,
  ChevronDown,
  ExternalLink,
  Loader2,
  XCircle,
} from 'lucide-react'
import { type FC, useEffect, useMemo, useRef, useState } from 'react'
import { useForm } from 'react-hook-form'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Command,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import {
  Form,
  FormControl,
  FormDescription,
  FormField,
  FormItem,
  FormLabel,
  FormMessage,
} from '@/components/ui/form'
import { Input } from '@/components/ui/input'
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from '@/components/ui/popover'
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from '@/components/ui/select'
import { Feature } from '@/lib/browseros/capabilities'
import {
  AI_PROVIDER_ADDED_EVENT,
  AI_PROVIDER_UPDATED_EVENT,
  KIMI_API_KEY_CONFIGURED_EVENT,
  KIMI_API_KEY_GUIDE_CLICKED_EVENT,
  MODEL_SELECTED_EVENT,
} from '@/lib/constants/analyticsEvents'
import {
  getDefaultBaseUrlForProviders,
  getProviderTemplate,
  providerTypeOptions,
} from '@/lib/llm-providers/providerTemplates'
import { type TestResult, testProvider } from '@/lib/llm-providers/testProvider'
import type { LlmProviderConfig, ProviderType } from '@/lib/llm-providers/types'
import { track } from '@/lib/metrics/track'
import { cn } from '@/lib/utils'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import {
  getIncompleteCatalogHint,
  getModelPickerRows,
} from './model-picker.helpers'
import {
  getModelContextLength,
  getModelsForProvider,
  getReasoningEffortOptions,
  type ModelInfo,
  modelSupportsReasoning,
} from './models'
import {
  isCredentiallessProviderType,
  normalizeProviderFormValues,
  type ProviderFormValues,
  providerFormSchema,
} from './provider-form-schema'

/** Window assumed for any model the bundled catalog cannot size. */
const DEFAULT_CONTEXT_WINDOW = 128000

function defaultReasoningEffort(type?: ProviderType) {
  return type === 'chatgpt-pro' ? 'medium' : 'high'
}

/**
 * Valid temperature range by provider. models.dev only says whether temperature
 * is supported, not its range, so this encodes the provider-level limits.
 * Anthropic caps at 1.0 (the SDK clamps anything higher); most others accept 0-2.
 */
function getTemperatureRange(type?: ProviderType): {
  min: number
  max: number
} {
  if (type === 'anthropic') return { min: 0, max: 1 }
  return { min: 0, max: 2 }
}

/** Picks a sensible default effort from a model's allowed levels. */
function pickDefaultEffort(options: string[]): string {
  if (options.includes('medium')) return 'medium'
  if (options.includes('high')) return 'high'
  return options[Math.floor(options.length / 2)] ?? 'medium'
}

function formatContextWindow(tokens: number): string {
  if (tokens >= 1000000)
    return `${(tokens / 1000000).toFixed(tokens % 1000000 === 0 ? 0 : 1)}M`
  if (tokens >= 1000) return `${Math.round(tokens / 1000)}K`
  return `${tokens}`
}

function setupGuideLabel(type: ProviderType, providerName?: string): string {
  if (type === 'moonshot') return 'How to get a Kimi API key'
  return providerName ? `${providerName} setup guide` : 'Provider setup guide'
}

function isProviderTypeOptionSupported(
  value: ProviderType,
  supports: (feature: Feature) => boolean,
): boolean {
  if (value === 'chatgpt-pro') return supports(Feature.CHATGPT_PRO_SUPPORT)
  if (value === 'github-copilot')
    return supports(Feature.GITHUB_COPILOT_SUPPORT)
  if (value === 'qwen-code') return supports(Feature.QWEN_CODE_SUPPORT)
  return true
}

function getVisibleProviderTypeOptions(
  supports: (feature: Feature) => boolean,
) {
  return providerTypeOptions.filter((opt) =>
    isProviderTypeOptionSupported(opt.value, supports),
  )
}

function isProviderTestable(input: {
  type: ProviderType
  modelId: string
  baseUrl?: string
  apiKey?: string
  resourceName?: string
  accessKeyId?: string
  secretAccessKey?: string
  region?: string
}): boolean {
  if (!input.modelId) return false

  if (
    input.type === 'chatgpt-pro' ||
    input.type === 'github-copilot' ||
    input.type === 'qwen-code'
  ) {
    return true
  }

  if (input.type === 'azure') {
    return Boolean((input.resourceName || input.baseUrl) && input.apiKey)
  }
  if (input.type === 'bedrock') {
    return Boolean(input.accessKeyId && input.secretAccessKey && input.region)
  }
  if (!input.baseUrl) return false
  if (!['ollama', 'lmstudio'].includes(input.type) && !input.apiKey) {
    return false
  }
  return true
}

export interface NewProviderDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  initialValues?: Partial<LlmProviderConfig>
  onSave: (provider: LlmProviderConfig) => Promise<void>
}

export const NewProviderDialog: FC<NewProviderDialogProps> = ({
  open,
  onOpenChange,
  initialValues,
  onSave,
}) => {
  const [isTesting, setIsTesting] = useState(false)
  const [testResult, setTestResult] = useState<TestResult | null>(null)
  const [modelPickerOpen, setModelPickerOpen] = useState(false)
  const [modelSearch, setModelSearch] = useState('')
  const modelListRef = useRef<HTMLDivElement>(null)
  const { supports } = useCapabilities()
  const { baseUrl: agentServerUrl } = useAgentServerUrl()

  const filteredProviderTypeOptions = getVisibleProviderTypeOptions(supports)

  const form = useForm<ProviderFormValues>({
    resolver: zodResolver(providerFormSchema),
    defaultValues: {
      type: initialValues?.type || 'openai',
      name: initialValues?.name || '',
      baseUrl:
        initialValues?.baseUrl || getDefaultBaseUrlForProviders('openai'),
      modelId: initialValues?.modelId || '',
      apiKey: initialValues?.apiKey || '',
      supportsImages: initialValues?.supportsImages ?? false,
      contextWindow: initialValues?.contextWindow || DEFAULT_CONTEXT_WINDOW,
      temperature: initialValues?.temperature ?? 0.2,
      resourceName: initialValues?.resourceName || '',
      accessKeyId: initialValues?.accessKeyId || '',
      secretAccessKey: initialValues?.secretAccessKey || '',
      region: initialValues?.region || '',
      sessionToken: initialValues?.sessionToken || '',
      reasoningEffort:
        initialValues?.reasoningEffort ||
        defaultReasoningEffort(initialValues?.type),
      reasoningSummary: initialValues?.reasoningSummary || 'auto',
    },
  })

  const watchedType = form.watch('type')
  const watchedModelId = form.watch('modelId')

  const watchedApiKey = form.watch('apiKey')
  const watchedBaseUrl = form.watch('baseUrl')
  const watchedResourceName = form.watch('resourceName')
  const watchedAccessKeyId = form.watch('accessKeyId')
  const watchedSecretAccessKey = form.watch('secretAccessKey')
  const watchedRegion = form.watch('region')
  const watchedSessionToken = form.watch('sessionToken')

  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional - clear result when any credential changes
  useEffect(() => {
    setTestResult(null)
  }, [
    watchedType,
    watchedModelId,
    watchedApiKey,
    watchedBaseUrl,
    watchedResourceName,
    watchedAccessKeyId,
    watchedSecretAccessKey,
    watchedRegion,
    watchedSessionToken,
  ])

  const modelInfoList = getModelsForProvider(watchedType as ProviderType)
  const selectedModel: ModelInfo | undefined = modelInfoList.find(
    (m) => m.modelId === watchedModelId,
  )
  const showReasoning = modelSupportsReasoning(
    selectedModel,
    watchedType as ProviderType,
  )
  const reasoningEffortOptions = getReasoningEffortOptions(selectedModel)
  const temperatureDisabled = selectedModel?.supportsTemperature === false
  const temperatureRange = getTemperatureRange(watchedType as ProviderType)

  // Context window guardrails for catalog models with a known window.
  const modelDefaultContext = selectedModel?.contextLength
  const watchedContextWindow = form.watch('contextWindow')
  const contextIsCustom =
    modelDefaultContext !== undefined &&
    watchedContextWindow !== modelDefaultContext
  const contextExceedsMax =
    modelDefaultContext !== undefined &&
    typeof watchedContextWindow === 'number' &&
    watchedContextWindow > modelDefaultContext
  const resetContextWindow = () => {
    if (modelDefaultContext !== undefined) {
      form.setValue('contextWindow', modelDefaultContext)
    }
  }
  const resetContextLink = (
    <button
      type="button"
      onClick={resetContextWindow}
      className="cursor-pointer text-primary hover:underline"
    >
      Reset
    </button>
  )

  const modelFuse = useMemo(
    () =>
      new Fuse(modelInfoList, {
        keys: ['modelId'],
        threshold: 0.4,
        distance: 100,
      }),
    [modelInfoList],
  )

  const { customModelId, models: filteredModels } = getModelPickerRows(
    modelSearch,
    modelInfoList,
    (query) => modelFuse.search(query).map((r) => r.item),
  )

  const commitModelId = (modelId: string, contextLength?: number) => {
    form.setValue('modelId', modelId)
    const info = modelInfoList.find((m) => m.modelId === modelId)
    if (info?.supportsImages !== undefined) {
      form.setValue('supportsImages', info.supportsImages)
    }
    // Reset effort to a level this model actually supports.
    form.setValue(
      'reasoningEffort',
      pickDefaultEffort(getReasoningEffortOptions(info)),
    )
    track(MODEL_SELECTED_EVENT, {
      provider_type: watchedType,
      model_id: modelId,
      ...(contextLength === undefined ? {} : { context_window: contextLength }),
      is_custom_model: !modelInfoList.some((m) => m.modelId === modelId),
    })
    setModelPickerOpen(false)
    setModelSearch('')
  }

  const handleTypeChange = (newType: ProviderType) => {
    form.setValue('type', newType)
    form.setValue('baseUrl', getDefaultBaseUrlForProviders(newType))
    form.setValue('reasoningEffort', defaultReasoningEffort(newType))
    form.setValue('modelId', '')
  }

  useEffect(() => {
    if (initialValues?.id) return
    if (!watchedModelId) return

    // A custom model has no catalog entry, so fall back to the default rather
    // than keeping whatever the previously selected model left behind: picking
    // gpt-5.5 and then pasting an 8k local model would otherwise save a
    // 1M-token window and overflow it on the first long chat.
    const contextLength = getModelContextLength(
      watchedType as ProviderType,
      watchedModelId,
    )
    form.setValue('contextWindow', contextLength || DEFAULT_CONTEXT_WINDOW)
  }, [watchedModelId, watchedType, form, initialValues?.id])

  useEffect(() => {
    if (initialValues) {
      form.reset({
        type: initialValues.type || 'openai',
        name: initialValues.name || '',
        baseUrl:
          initialValues.baseUrl ||
          getDefaultBaseUrlForProviders(initialValues.type || 'openai'),
        modelId: initialValues.modelId || '',
        apiKey: initialValues.apiKey || '',
        supportsImages: initialValues.supportsImages ?? false,
        contextWindow: initialValues.contextWindow || DEFAULT_CONTEXT_WINDOW,
        temperature: initialValues.temperature ?? 0.2,
        resourceName: initialValues.resourceName || '',
        accessKeyId: initialValues.accessKeyId || '',
        secretAccessKey: initialValues.secretAccessKey || '',
        region: initialValues.region || '',
        sessionToken: initialValues.sessionToken || '',
        reasoningEffort:
          initialValues.reasoningEffort ||
          defaultReasoningEffort(initialValues.type),
        reasoningSummary: initialValues.reasoningSummary || 'auto',
      })
    }
  }, [initialValues, form])

  useEffect(() => {
    if (open && !initialValues) {
      const defaultType = 'openai'
      form.reset({
        type: defaultType,
        name: '',
        baseUrl: getDefaultBaseUrlForProviders(defaultType),
        modelId: '',
        apiKey: '',
        supportsImages: false,
        contextWindow: DEFAULT_CONTEXT_WINDOW,
        temperature: 0.2,
        resourceName: '',
        accessKeyId: '',
        secretAccessKey: '',
        region: '',
        sessionToken: '',
        reasoningEffort: defaultReasoningEffort(defaultType),
        reasoningSummary: 'auto',
      })
    }
    setTestResult(null)
  }, [open, initialValues, form])

  const onSubmit = async (values: ProviderFormValues) => {
    const isNewProvider = !initialValues?.id
    const normalizedValues = normalizeProviderFormValues(values)
    const provider: LlmProviderConfig = {
      id: initialValues?.id || crypto.randomUUID(),
      ...normalizedValues,
      createdAt: initialValues?.createdAt || Date.now(),
      updatedAt: Date.now(),
    }

    await onSave(provider)
    if (isNewProvider) {
      track(AI_PROVIDER_ADDED_EVENT, {
        provider_type: normalizedValues.type,
        model: normalizedValues.modelId,
      })
    } else {
      track(AI_PROVIDER_UPDATED_EVENT, {
        provider_type: normalizedValues.type,
        model: normalizedValues.modelId,
      })
    }
    if (normalizedValues.type === 'moonshot') {
      track(KIMI_API_KEY_CONFIGURED_EVENT, {
        model: normalizedValues.modelId,
        is_new: isNewProvider,
      })
    }
    form.reset()
    onOpenChange(false)
  }

  const canTest = isProviderTestable({
    type: watchedType as ProviderType,
    modelId: watchedModelId,
    baseUrl: watchedBaseUrl,
    apiKey: watchedApiKey,
    resourceName: watchedResourceName,
    accessKeyId: watchedAccessKeyId,
    secretAccessKey: watchedSecretAccessKey,
    region: watchedRegion,
  })

  const handleTest = async () => {
    if (!agentServerUrl) {
      setTestResult({
        success: false,
        message: 'Server URL not available',
      })
      return
    }

    setIsTesting(true)
    setTestResult(null)

    try {
      const values = form.getValues()

      const result = await testProvider(
        {
          id: 'test',
          type: values.type,
          name: values.name || 'Test',
          baseUrl: values.baseUrl,
          modelId: values.modelId,
          apiKey: values.apiKey,
          supportsImages: values.supportsImages,
          contextWindow: values.contextWindow,
          temperature: values.temperature,
          createdAt: Date.now(),
          updatedAt: Date.now(),
          resourceName: values.resourceName,
          accessKeyId: values.accessKeyId,
          secretAccessKey: values.secretAccessKey,
          region: values.region,
          sessionToken: values.sessionToken,
        },
        agentServerUrl,
      )

      setTestResult(result)
    } catch (error) {
      setTestResult({
        success: false,
        message: error instanceof Error ? error.message : 'Test failed',
      })
    } finally {
      setIsTesting(false)
    }
  }

  const providerTemplate = getProviderTemplate(watchedType as ProviderType)
  const setupGuideUrl = providerTemplate?.setupGuideUrl
  const providerName = providerTemplate?.name
  const setupGuideText = setupGuideLabel(
    watchedType as ProviderType,
    providerName,
  )
  const modelCatalogHint = getIncompleteCatalogHint(
    watchedType as ProviderType,
    modelInfoList.length,
    providerName,
  )

  const handleSetupGuideClick = (e: React.MouseEvent) => {
    e.preventDefault()
    if (watchedType === 'moonshot') {
      track(KIMI_API_KEY_GUIDE_CLICKED_EVENT)
    }
    if (setupGuideUrl) chrome.tabs.create({ url: setupGuideUrl })
  }

  // Reasoning summaries are an OpenAI-family concept; other providers stream
  // reasoning text without a separate summary control.
  const showReasoningSummary =
    watchedType === 'openai' ||
    watchedType === 'azure' ||
    watchedType === 'chatgpt-pro'

  const renderReasoningControls = () => (
    <div className="space-y-4 border-border border-t pt-4">
      <h4 className="font-medium text-sm">Reasoning</h4>
      <div className="grid gap-4 sm:grid-cols-2">
        <FormField
          control={form.control}
          name="reasoningEffort"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Reasoning Effort</FormLabel>
              <Select
                onValueChange={field.onChange}
                value={
                  reasoningEffortOptions.includes(field.value ?? '')
                    ? field.value
                    : pickDefaultEffort(reasoningEffortOptions)
                }
              >
                <FormControl>
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                </FormControl>
                <SelectContent>
                  {reasoningEffortOptions.map((value) => (
                    <SelectItem key={value} value={value}>
                      {value.charAt(0).toUpperCase() + value.slice(1)}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <FormDescription>
                How much the model thinks before responding
              </FormDescription>
              <FormMessage />
            </FormItem>
          )}
        />
        {showReasoningSummary && (
          <FormField
            control={form.control}
            name="reasoningSummary"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Reasoning Summary</FormLabel>
                <Select
                  onValueChange={field.onChange}
                  value={field.value || 'auto'}
                >
                  <FormControl>
                    <SelectTrigger className="w-full">
                      <SelectValue />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    <SelectItem value="auto">Auto</SelectItem>
                    <SelectItem value="concise">Concise</SelectItem>
                    <SelectItem value="detailed">Detailed</SelectItem>
                  </SelectContent>
                </Select>
                <FormDescription>
                  Detail level of visible thinking steps
                </FormDescription>
                <FormMessage />
              </FormItem>
            )}
          />
        )}
      </div>
    </div>
  )

  const renderProviderSpecificFields = () => {
    if (
      isCredentiallessProviderType(watchedType) &&
      watchedType !== 'chatgpt-pro'
    ) {
      const name = watchedType === 'github-copilot' ? 'GitHub' : 'Qwen Code'
      return (
        <div className="rounded-lg border border-green-200 bg-green-50 p-3 text-green-700 text-sm dark:border-green-800 dark:bg-green-950 dark:text-green-300">
          Credentials are managed via {name} OAuth. No API key needed.
        </div>
      )
    }

    if (watchedType === 'chatgpt-pro') {
      return (
        <div className="rounded-lg border border-green-200 bg-green-50 p-3 text-green-700 text-sm dark:border-green-800 dark:bg-green-950 dark:text-green-300">
          Credentials are managed via OAuth. No API key needed.
        </div>
      )
    }

    if (watchedType === 'azure') {
      return (
        <>
          <div className="grid gap-4 sm:grid-cols-2">
            <FormField
              control={form.control}
              name="resourceName"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Resource Name *</FormLabel>
                  <FormControl>
                    <Input placeholder="your-resource-name" {...field} />
                  </FormControl>
                  <FormDescription>Azure OpenAI resource name</FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="baseUrl"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Base URL Override</FormLabel>
                  <FormControl>
                    <Input placeholder="Optional custom URL" {...field} />
                  </FormControl>
                  <FormDescription>
                    Overrides resource name if set
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>
          <FormField
            control={form.control}
            name="apiKey"
            render={({ field }) => (
              <FormItem>
                <FormLabel>API Key *</FormLabel>
                <FormControl>
                  <Input
                    type="password"
                    placeholder="Enter your Azure API key"
                    {...field}
                  />
                </FormControl>
                <FormMessage />
              </FormItem>
            )}
          />
        </>
      )
    }

    if (watchedType === 'bedrock') {
      return (
        <>
          <div className="grid gap-4 sm:grid-cols-2">
            <FormField
              control={form.control}
              name="accessKeyId"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Access Key ID *</FormLabel>
                  <FormControl>
                    <Input placeholder="AKIA..." {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="secretAccessKey"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Secret Access Key *</FormLabel>
                  <FormControl>
                    <Input
                      type="password"
                      placeholder="Enter your secret access key"
                      {...field}
                    />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <FormField
              control={form.control}
              name="region"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Region *</FormLabel>
                  <FormControl>
                    <Input placeholder="us-east-1" {...field} />
                  </FormControl>
                  <FormMessage />
                </FormItem>
              )}
            />
            <FormField
              control={form.control}
              name="sessionToken"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Session Token</FormLabel>
                  <FormControl>
                    <Input
                      type="password"
                      placeholder="Optional (for STS credentials)"
                      {...field}
                    />
                  </FormControl>
                  <FormDescription>
                    Required for temporary credentials
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />
          </div>
        </>
      )
    }

    return (
      <>
        <FormField
          control={form.control}
          name="baseUrl"
          render={({ field }) => (
            <FormItem>
              <FormLabel>Base URL *</FormLabel>
              <FormControl>
                <Input placeholder="https://api.openai.com/v1" {...field} />
              </FormControl>
              {watchedType === 'openai' && (
                <FormDescription>
                  If your custom endpoint doesn't work with OpenAI, try the
                  OpenAI Compatible provider template instead.
                </FormDescription>
              )}
              <FormMessage />
            </FormItem>
          )}
        />
        <FormField
          control={form.control}
          name="apiKey"
          render={({ field }) => {
            const isApiKeyOptional = ['ollama', 'lmstudio'].includes(
              watchedType,
            )
            return (
              <FormItem>
                <FormLabel>API Key{isApiKeyOptional ? '' : ' *'}</FormLabel>
                <FormControl>
                  <Input
                    type="password"
                    placeholder={
                      isApiKeyOptional
                        ? 'Enter your API key (optional)'
                        : 'Enter your API key'
                    }
                    {...field}
                  />
                </FormControl>
                <FormDescription>
                  Your API key is encrypted and stored locally.{' '}
                  {setupGuideUrl && (
                    <a
                      href={setupGuideUrl}
                      onClick={handleSetupGuideClick}
                      className="inline-flex cursor-pointer items-center gap-1 text-primary hover:underline"
                    >
                      <ExternalLink className="h-3 w-3" />
                      {setupGuideText}
                    </a>
                  )}
                </FormDescription>
                <FormMessage />
              </FormItem>
            )
          }}
        />
      </>
    )
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {initialValues?.id ? 'Edit Provider' : 'Configure New Provider'}
          </DialogTitle>
          <DialogDescription>
            {initialValues?.id
              ? 'Update your LLM provider configuration.'
              : 'Add a new LLM provider configuration with API key and model settings.'}
          </DialogDescription>
        </DialogHeader>
        <Form {...form}>
          <form onSubmit={form.handleSubmit(onSubmit)} className="space-y-5">
            <div className="grid gap-4 sm:grid-cols-2">
              <FormField
                control={form.control}
                name="type"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Provider Type *</FormLabel>
                    <Select
                      onValueChange={(v) => handleTypeChange(v as ProviderType)}
                      value={field.value}
                    >
                      <FormControl>
                        <SelectTrigger className="w-full">
                          <SelectValue placeholder="Select provider type" />
                        </SelectTrigger>
                      </FormControl>
                      <SelectContent>
                        {filteredProviderTypeOptions.map((opt) => (
                          <SelectItem key={opt.value} value={opt.value}>
                            {opt.label}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                    <FormMessage />
                  </FormItem>
                )}
              />
              <FormField
                control={form.control}
                name="name"
                render={({ field }) => (
                  <FormItem>
                    <FormLabel>Provider Name *</FormLabel>
                    <FormControl>
                      <Input placeholder="e.g., Work OpenAI" {...field} />
                    </FormControl>
                    <FormMessage />
                  </FormItem>
                )}
              />
            </div>

            {renderProviderSpecificFields()}

            <FormField
              control={form.control}
              name="modelId"
              render={({ field }) => (
                <FormItem className="flex flex-col">
                  <FormLabel>Model *</FormLabel>
                  {modelInfoList.length === 0 ? (
                    <FormControl>
                      <Input
                        placeholder={
                          watchedType === 'azure'
                            ? 'Enter your deployment name'
                            : watchedType === 'bedrock'
                              ? 'e.g., anthropic.claude-3-5-sonnet-20241022-v2:0'
                              : 'Enter model ID'
                        }
                        {...field}
                      />
                    </FormControl>
                  ) : (
                    <Popover
                      // modal makes this popover own the scroll lock while open.
                      // Without it the Dialog's react-remove-scroll blocks wheel
                      // events over the body-portaled list, so it cannot scroll
                      // past its max height (only search could reach lower rows).
                      modal
                      open={modelPickerOpen}
                      onOpenChange={(isOpen) => {
                        setModelPickerOpen(isOpen)
                        if (!isOpen) setModelSearch('')
                      }}
                    >
                      <PopoverTrigger asChild>
                        <button
                          type="button"
                          className={cn(
                            'flex h-9 w-full items-center justify-between rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs',
                            field.value
                              ? 'text-foreground'
                              : 'text-muted-foreground',
                          )}
                        >
                          <span className="truncate">
                            {field.value || 'Select or paste a model ID'}
                          </span>
                          <ChevronDown className="ml-2 h-4 w-4 shrink-0 opacity-50" />
                        </button>
                      </PopoverTrigger>
                      <PopoverContent
                        className="w-[var(--radix-popover-trigger-width)] p-0"
                        align="start"
                      >
                        <Command shouldFilter={false}>
                          <CommandInput
                            placeholder="Search or paste a model ID..."
                            value={modelSearch}
                            onValueChange={(v) => {
                              setModelSearch(v)
                              requestAnimationFrame(() => {
                                modelListRef.current?.scrollTo(0, 0)
                              })
                            }}
                          />
                          <CommandList ref={modelListRef}>
                            {customModelId !== null && (
                              <CommandGroup forceMount>
                                <CommandItem
                                  forceMount
                                  value={`custom:${customModelId}`}
                                  onSelect={() => commitModelId(customModelId)}
                                >
                                  <span className="flex-1 truncate">
                                    Use &quot;{customModelId}&quot;
                                  </span>
                                  <span className="ml-2 shrink-0 rounded-md bg-muted px-1.5 py-0.5 text-[10px] text-muted-foreground uppercase tracking-wide">
                                    Custom
                                  </span>
                                  {field.value === customModelId && (
                                    <Check className="ml-2 h-4 w-4 shrink-0" />
                                  )}
                                </CommandItem>
                              </CommandGroup>
                            )}
                            {filteredModels.length > 0 && (
                              <CommandGroup>
                                {filteredModels.map((model) => (
                                  <CommandItem
                                    key={model.modelId}
                                    value={model.modelId}
                                    onSelect={() =>
                                      commitModelId(
                                        model.modelId,
                                        model.contextLength,
                                      )
                                    }
                                  >
                                    <span className="flex-1 truncate">
                                      {model.modelId}
                                    </span>
                                    {model.contextLength > 0 && (
                                      <span className="ml-2 shrink-0 rounded-md bg-muted px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground">
                                        {formatContextWindow(
                                          model.contextLength,
                                        )}
                                      </span>
                                    )}
                                    {field.value === model.modelId && (
                                      <Check className="ml-2 h-4 w-4 shrink-0" />
                                    )}
                                  </CommandItem>
                                ))}
                              </CommandGroup>
                            )}
                          </CommandList>
                          {/* cmdk re-selects the first item on every
                                keystroke, so the free-form row above is what
                                Enter commits until the user arrows away. */}
                          <p className="border-border border-t px-3 py-2 text-[11px] text-muted-foreground">
                            Model not listed? Type or paste its exact ID, then
                            press Enter.
                          </p>
                        </Command>
                      </PopoverContent>
                    </Popover>
                  )}
                  {modelCatalogHint && (
                    <FormDescription>{modelCatalogHint}</FormDescription>
                  )}
                  <FormMessage />
                </FormItem>
              )}
            />

            {showReasoning && renderReasoningControls()}

            <div className="space-y-4 border-border border-t pt-4">
              <h4 className="font-medium text-sm">Model Configuration</h4>
              <FormField
                control={form.control}
                name="supportsImages"
                render={({ field }) => (
                  <FormItem className="flex items-center gap-2 space-y-0">
                    <FormControl>
                      <Checkbox
                        checked={field.value}
                        onCheckedChange={field.onChange}
                      />
                    </FormControl>
                    <FormLabel className="font-normal">
                      Supports Images
                    </FormLabel>
                  </FormItem>
                )}
              />
              <div className="grid gap-4 sm:grid-cols-2">
                <FormField
                  control={form.control}
                  name="contextWindow"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Context Window Size</FormLabel>
                      <FormControl>
                        <Input
                          type="number"
                          aria-invalid={contextExceedsMax}
                          {...field}
                          onChange={(e) =>
                            field.onChange(Number(e.target.value))
                          }
                        />
                      </FormControl>
                      {contextExceedsMax && (
                        <p className="text-destructive text-sm">
                          Context window cannot exceed{' '}
                          {formatContextWindow(modelDefaultContext ?? 0)}.{' '}
                          {resetContextLink}
                        </p>
                      )}
                      {!contextExceedsMax && contextIsCustom && (
                        <FormDescription>
                          Custom value added. {resetContextLink}
                        </FormDescription>
                      )}
                      {!contextExceedsMax && !contextIsCustom && (
                        <FormDescription>
                          Auto-filled based on model
                        </FormDescription>
                      )}
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="temperature"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>
                        Temperature ({temperatureRange.min}-
                        {temperatureRange.max})
                      </FormLabel>
                      <FormControl>
                        <Input
                          type="number"
                          step="0.1"
                          min={temperatureRange.min}
                          max={temperatureRange.max}
                          disabled={temperatureDisabled}
                          {...field}
                          onChange={(e) =>
                            field.onChange(Number(e.target.value))
                          }
                        />
                      </FormControl>
                      <FormDescription>
                        {temperatureDisabled
                          ? 'This model does not support temperature'
                          : 'Controls response randomness'}
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
              </div>
            </div>

            {testResult && (
              <div
                className={`flex items-center gap-2 rounded-lg border p-3 text-sm ${
                  testResult.success
                    ? 'border-green-200 bg-green-50 text-green-700 dark:border-green-800 dark:bg-green-950 dark:text-green-300'
                    : 'border-red-200 bg-red-50 text-red-700 dark:border-red-800 dark:bg-red-950 dark:text-red-300'
                }`}
              >
                {testResult.success ? (
                  <CheckCircle2 className="h-4 w-4 shrink-0" />
                ) : (
                  <XCircle className="h-4 w-4 shrink-0" />
                )}
                <span className="flex-1">{testResult.message}</span>
                {testResult.responseTime && (
                  <span className="text-xs opacity-70">
                    {testResult.responseTime}ms
                  </span>
                )}
              </div>
            )}

            <DialogFooter className="gap-3">
              <Button
                type="button"
                variant="outline"
                onClick={() => onOpenChange(false)}
              >
                Cancel
              </Button>
              <Button
                type="button"
                variant="outline"
                onClick={handleTest}
                disabled={!canTest || isTesting}
              >
                {isTesting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                {isTesting ? 'Testing...' : 'Test'}
              </Button>
              <Button type="submit" disabled={isTesting || contextExceedsMax}>
                {initialValues?.id ? 'Update' : 'Save'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
