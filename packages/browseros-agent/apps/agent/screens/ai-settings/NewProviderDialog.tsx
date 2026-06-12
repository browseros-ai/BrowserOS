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
  CommandEmpty,
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
import { isLocalRuntimeProviderType } from '@/lib/llm-providers/provider-runtime'
import {
  getDefaultBaseUrlForProviders,
  getProviderTemplate,
  providerTypeOptions,
} from '@/lib/llm-providers/providerTemplates'
import { type TestResult, testProvider } from '@/lib/llm-providers/testProvider'
import {
  type LlmProviderConfig,
  type ProviderType,
  REMOTE_HERMES_PROVIDER_TYPE,
} from '@/lib/llm-providers/types'
import { track } from '@/lib/metrics/track'
import { cn } from '@/lib/utils'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'
import { useCapabilities } from '@/modules/browseros/capabilities.hooks'
import { useAcpProbe } from '@/modules/llm-providers/acp-probe.hooks'
import { getModelContextLength, getModelsForProvider } from './models'
import {
  isCredentiallessProviderType,
  normalizeProviderFormValues,
  type ProviderFormValues,
  providerFormSchema,
} from './provider-form-schema'

const ACP_PROVIDER_TYPES = new Set<ProviderType>([
  'claude-code',
  'codex',
  'acp-custom',
])

function isAcpProviderType(type: ProviderType | undefined): boolean {
  return type !== undefined && ACP_PROVIDER_TYPES.has(type)
}

function isRemoteHermesType(type: ProviderType | undefined): boolean {
  return type === REMOTE_HERMES_PROVIDER_TYPE
}

const EFFORT_LABEL: Record<string, string> = {
  none: 'None',
  low: 'Low',
  medium: 'Medium',
  high: 'High',
  xhigh: 'Extra High',
  max: 'Max',
}

function formatContextWindow(tokens: number): string {
  if (tokens >= 1000000)
    return `${(tokens / 1000000).toFixed(tokens % 1000000 === 0 ? 0 : 1)}M`
  if (tokens >= 1000) return `${Math.round(tokens / 1000)}K`
  return `${tokens}`
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

  const filteredProviderTypeOptions = providerTypeOptions.filter((opt) => {
    if (opt.value === 'chatgpt-pro')
      return supports(Feature.CHATGPT_PRO_SUPPORT)
    if (opt.value === 'github-copilot')
      return supports(Feature.GITHUB_COPILOT_SUPPORT)
    if (opt.value === 'qwen-code') return supports(Feature.QWEN_CODE_SUPPORT)
    if (opt.value === 'openai-compatible') {
      return supports(Feature.OPENAI_COMPATIBLE_SUPPORT)
    }
    return true
  })

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
      contextWindow: initialValues?.contextWindow || 128000,
      temperature: initialValues?.temperature ?? 0.2,
      resourceName: initialValues?.resourceName || '',
      accessKeyId: initialValues?.accessKeyId || '',
      secretAccessKey: initialValues?.secretAccessKey || '',
      region: initialValues?.region || '',
      sessionToken: initialValues?.sessionToken || '',
      reasoningEffort: initialValues?.reasoningEffort || 'high',
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

  const acpProbe = useAcpProbe({
    providerType: watchedType as ProviderType,
    enabled: isAcpProviderType(watchedType as ProviderType),
  })

  // Probe data arrives asynchronously after the dialog mounts; when it
  // resolves and the form's modelId / reasoningEffort fields are still
  // empty, default them to the probe's first settable model and the
  // agent's reported defaultEffort. External-system response, not state
  // derivation from props (CLAUDE.md useEffect carve-out applies).
  useEffect(() => {
    if (!isAcpProviderType(watchedType as ProviderType)) return
    if (!acpProbe.data) return
    const firstModel = acpProbe.data.models[0]?.id
    if (firstModel && !form.getValues('modelId')) {
      form.setValue('modelId', firstModel, { shouldDirty: false })
    }
    const defaultEffort = acpProbe.data.reasoning?.defaultValue
    if (defaultEffort && !form.getValues('reasoningEffort')) {
      form.setValue(
        'reasoningEffort',
        defaultEffort as ProviderFormValues['reasoningEffort'],
        { shouldDirty: false },
      )
    }
  }, [acpProbe.data, watchedType, form])

  const modelInfoList = getModelsForProvider(watchedType as ProviderType)

  const modelFuse = useMemo(
    () =>
      new Fuse(modelInfoList, {
        keys: ['modelId'],
        threshold: 0.4,
        distance: 100,
      }),
    [modelInfoList],
  )

  const filteredModels = modelSearch
    ? modelFuse.search(modelSearch).map((r) => r.item)
    : modelInfoList

  const showCustomEntry =
    modelSearch && !filteredModels.some((m) => m.modelId === modelSearch)

  const handleTypeChange = (newType: ProviderType) => {
    form.setValue('type', newType)
    form.setValue('baseUrl', getDefaultBaseUrlForProviders(newType))
    if (isLocalRuntimeProviderType(newType)) {
      form.setValue('apiKey', '')
      form.setValue('resourceName', '')
      form.setValue('accessKeyId', '')
      form.setValue('secretAccessKey', '')
      form.setValue('region', '')
      form.setValue('sessionToken', '')
    }
    if (isRemoteHermesType(newType)) {
      form.setValue('apiKey', '')
      form.setValue('modelId', 'default')
      if (!form.getValues('name')) form.setValue('name', 'Remote Hermes')
      return
    }
    form.setValue('modelId', '')
  }

  useEffect(() => {
    if (initialValues?.id) return

    if (watchedModelId) {
      const contextLength = getModelContextLength(
        watchedType as ProviderType,
        watchedModelId,
      )
      if (contextLength) {
        form.setValue('contextWindow', contextLength)
      }
    }
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
        contextWindow: initialValues.contextWindow || 128000,
        temperature: initialValues.temperature ?? 0.2,
        resourceName: initialValues.resourceName || '',
        accessKeyId: initialValues.accessKeyId || '',
        secretAccessKey: initialValues.secretAccessKey || '',
        region: initialValues.region || '',
        sessionToken: initialValues.sessionToken || '',
        reasoningEffort: initialValues.reasoningEffort || 'high',
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
        contextWindow: 128000,
        temperature: 0.2,
        resourceName: '',
        accessKeyId: '',
        secretAccessKey: '',
        region: '',
        sessionToken: '',
        reasoningEffort: 'high',
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

  const canTest = (): boolean => {
    if (!watchedModelId) return false

    if (isAcpProviderType(watchedType as ProviderType)) {
      // acp-custom must carry a command for the probe to spawn anything;
      // built-ins resolve their command through acpx's registry.
      if (watchedType === 'acp-custom' && !form.getValues('acpCommand')) {
        return false
      }
      return true
    }

    if (
      watchedType === 'chatgpt-pro' ||
      watchedType === 'github-copilot' ||
      watchedType === 'qwen-code'
    )
      return true

    if (watchedType === 'azure') {
      return !!(watchedResourceName || watchedBaseUrl) && !!watchedApiKey
    }
    if (watchedType === 'bedrock') {
      return !!watchedAccessKeyId && !!watchedSecretAccessKey && !!watchedRegion
    }
    if (!watchedBaseUrl) return false
    if (!['ollama', 'lmstudio'].includes(watchedType) && !watchedApiKey) {
      return false
    }
    return true
  }

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
  const setupGuideText =
    watchedType === 'moonshot'
      ? 'How to get a Kimi API key'
      : providerName
        ? `${providerName} setup guide`
        : 'Provider setup guide'

  const handleSetupGuideClick = (e: React.MouseEvent) => {
    e.preventDefault()
    if (watchedType === 'moonshot') {
      track(KIMI_API_KEY_GUIDE_CLICKED_EVENT)
    }
    if (setupGuideUrl) chrome.tabs.create({ url: setupGuideUrl })
  }

  const renderProviderSpecificFields = () => {
    if (isRemoteHermesType(watchedType as ProviderType)) {
      return (
        <div className="rounded-lg border border-green-200 bg-green-50 p-3 text-green-700 text-sm dark:border-green-800 dark:bg-green-950 dark:text-green-300">
          Remote Hermes runs in a managed VM. No API key, base URL, or model
          selection is required.
        </div>
      )
    }
    if (isAcpProviderType(watchedType as ProviderType)) {
      return renderAcpFields()
    }
    if (
      isCredentiallessProviderType(watchedType) &&
      watchedType !== 'chatgpt-pro'
    ) {
      const name =
        watchedType === 'github-copilot'
          ? 'GitHub'
          : watchedType === 'qwen-code'
            ? 'Qwen Code'
            : watchedType === 'codex'
              ? 'Codex'
              : 'Claude Code'
      const message =
        watchedType === 'codex' || watchedType === 'claude-code'
          ? `Credentials are managed by the local ${name} runtime. No API key needed.`
          : `Credentials are managed via ${name} OAuth. No API key needed.`
      return (
        <div className="rounded-lg border border-green-200 bg-green-50 p-3 text-green-700 text-sm dark:border-green-800 dark:bg-green-950 dark:text-green-300">
          {message}
        </div>
      )
    }

    function renderAcpFields() {
      const agentLabel = watchedType === 'codex' ? 'Codex' : 'Claude Code'
      const banner = (
        <div className="rounded-lg border border-green-200 bg-green-50 p-3 text-green-700 text-sm dark:border-green-800 dark:bg-green-950 dark:text-green-300">
          Credentials are managed by the local {agentLabel} runtime. No API key
          needed.
        </div>
      )
      if (acpProbe.isPending) {
        return (
          <>
            {banner}
            <div className="flex items-center gap-2 text-muted-foreground text-sm">
              <Loader2 className="size-4 animate-spin" />
              Probing local {agentLabel} for available models and effort
              levels...
            </div>
          </>
        )
      }
      if (acpProbe.isError) {
        return (
          <>
            {banner}
            <div className="rounded-lg border border-red-200 bg-red-50 p-3 text-red-700 text-sm dark:border-red-800 dark:bg-red-950 dark:text-red-300">
              <div>
                Could not probe {agentLabel}:{' '}
                {acpProbe.error instanceof Error
                  ? acpProbe.error.message
                  : String(acpProbe.error)}
              </div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="mt-2 h-7 px-2 text-xs"
                onClick={() => acpProbe.refetch()}
              >
                Retry
              </Button>
            </div>
          </>
        )
      }
      const probe = acpProbe.data
      if (probe?.error) {
        return (
          <>
            {banner}
            <div className="rounded-lg border border-red-200 bg-red-50 p-3 text-red-700 text-sm dark:border-red-800 dark:bg-red-950 dark:text-red-300">
              <div>
                {agentLabel} responded with{' '}
                <code className="rounded bg-red-100 px-1 dark:bg-red-900">
                  {probe.error.code}
                </code>
                : {probe.error.message}
              </div>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                className="mt-2 h-7 px-2 text-xs"
                onClick={() => acpProbe.refetch()}
              >
                Retry
              </Button>
            </div>
          </>
        )
      }
      const models = probe?.models ?? []
      const effortValues = probe?.reasoning?.values ?? []
      return (
        <>
          {banner}
          <FormField
            control={form.control}
            name="modelId"
            render={({ field }) => (
              <FormItem>
                <FormLabel>Model *</FormLabel>
                <Select
                  onValueChange={field.onChange}
                  value={field.value || ''}
                  disabled={models.length === 0}
                >
                  <FormControl>
                    <SelectTrigger className="w-full">
                      <SelectValue
                        placeholder={
                          models.length === 0
                            ? 'No settable models advertised'
                            : 'Select a model...'
                        }
                      />
                    </SelectTrigger>
                  </FormControl>
                  <SelectContent>
                    {models.map((m) => (
                      <SelectItem key={m.id} value={m.id}>
                        <span className="font-medium">{m.name ?? m.id}</span>
                        {m.description ? (
                          <span className="ml-2 text-muted-foreground text-xs">
                            {m.description}
                          </span>
                        ) : null}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
                <FormMessage />
              </FormItem>
            )}
          />
          {effortValues.length > 0 ? (
            <FormField
              control={form.control}
              name="reasoningEffort"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Reasoning Effort</FormLabel>
                  <Select
                    onValueChange={field.onChange}
                    value={field.value || ''}
                  >
                    <FormControl>
                      <SelectTrigger className="w-full">
                        <SelectValue placeholder="Select effort..." />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      {effortValues.map((value) => (
                        <SelectItem key={value} value={value}>
                          {EFFORT_LABEL[value] ?? value}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                  <FormMessage />
                </FormItem>
              )}
            />
          ) : null}
        </>
      )
    }

    if (watchedType === 'chatgpt-pro') {
      return (
        <>
          <div className="rounded-lg border border-green-200 bg-green-50 p-3 text-green-700 text-sm dark:border-green-800 dark:bg-green-950 dark:text-green-300">
            Credentials are managed via OAuth. No API key needed.
          </div>
          <div className="grid gap-4 sm:grid-cols-2">
            <FormField
              control={form.control}
              name="reasoningEffort"
              render={({ field }) => (
                <FormItem>
                  <FormLabel>Reasoning Effort</FormLabel>
                  <Select
                    onValueChange={field.onChange}
                    value={field.value || 'high'}
                  >
                    <FormControl>
                      <SelectTrigger className="w-full">
                        <SelectValue />
                      </SelectTrigger>
                    </FormControl>
                    <SelectContent>
                      <SelectItem value="none">None</SelectItem>
                      <SelectItem value="low">Low</SelectItem>
                      <SelectItem value="medium">Medium</SelectItem>
                      <SelectItem value="high">High</SelectItem>
                    </SelectContent>
                  </Select>
                  <FormDescription>
                    How much the model thinks before responding
                  </FormDescription>
                  <FormMessage />
                </FormItem>
              )}
            />
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
          </div>
        </>
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

            {!isAcpProviderType(watchedType as ProviderType) &&
              !isRemoteHermesType(watchedType as ProviderType) && (
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
                                {field.value || 'Select a model...'}
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
                                placeholder="Search models..."
                                value={modelSearch}
                                onValueChange={(v) => {
                                  setModelSearch(v)
                                  requestAnimationFrame(() => {
                                    modelListRef.current?.scrollTo(0, 0)
                                  })
                                }}
                                onKeyDown={(e) => {
                                  if (e.key === 'Enter' && modelSearch) {
                                    e.preventDefault()
                                    e.stopPropagation()
                                    form.setValue('modelId', modelSearch)
                                    track(MODEL_SELECTED_EVENT, {
                                      provider_type: watchedType,
                                      model_id: modelSearch,
                                      is_custom_model: !modelInfoList.some(
                                        (m) => m.modelId === modelSearch,
                                      ),
                                    })
                                    setModelPickerOpen(false)
                                    setModelSearch('')
                                  }
                                }}
                              />
                              <CommandList ref={modelListRef}>
                                <CommandEmpty>
                                  No models found. Press Enter to use &quot;
                                  {modelSearch}&quot;
                                </CommandEmpty>
                                {showCustomEntry && (
                                  <CommandGroup forceMount>
                                    <CommandItem
                                      forceMount
                                      value={`custom:${modelSearch}`}
                                      onSelect={() => {
                                        form.setValue('modelId', modelSearch)
                                        track(MODEL_SELECTED_EVENT, {
                                          provider_type: watchedType,
                                          model_id: modelSearch,
                                          is_custom_model: true,
                                        })
                                        setModelPickerOpen(false)
                                        setModelSearch('')
                                      }}
                                    >
                                      <span className="flex-1 truncate">
                                        {modelSearch}
                                      </span>
                                      {field.value === modelSearch && (
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
                                        onSelect={() => {
                                          form.setValue(
                                            'modelId',
                                            model.modelId,
                                          )
                                          track(MODEL_SELECTED_EVENT, {
                                            provider_type: watchedType,
                                            model_id: model.modelId,
                                            context_window: model.contextLength,
                                            is_custom_model:
                                              !modelInfoList.some(
                                                (m) =>
                                                  m.modelId === model.modelId,
                                              ),
                                          })
                                          setModelPickerOpen(false)
                                          setModelSearch('')
                                        }}
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
                            </Command>
                          </PopoverContent>
                        </Popover>
                      )}
                      <FormMessage />
                    </FormItem>
                  )}
                />
              )}

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
                          {...field}
                          onChange={(e) =>
                            field.onChange(Number(e.target.value))
                          }
                        />
                      </FormControl>
                      <FormDescription>
                        Auto-filled based on model
                      </FormDescription>
                      <FormMessage />
                    </FormItem>
                  )}
                />
                <FormField
                  control={form.control}
                  name="temperature"
                  render={({ field }) => (
                    <FormItem>
                      <FormLabel>Temperature (0-2)</FormLabel>
                      <FormControl>
                        <Input
                          type="number"
                          step="0.1"
                          min="0"
                          max="2"
                          {...field}
                          onChange={(e) =>
                            field.onChange(Number(e.target.value))
                          }
                        />
                      </FormControl>
                      <FormDescription>
                        Controls response randomness
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
                disabled={!canTest() || isTesting}
              >
                {isTesting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                {isTesting ? 'Testing...' : 'Test'}
              </Button>
              <Button type="submit" disabled={isTesting}>
                {initialValues?.id ? 'Update' : 'Save'}
              </Button>
            </DialogFooter>
          </form>
        </Form>
      </DialogContent>
    </Dialog>
  )
}
