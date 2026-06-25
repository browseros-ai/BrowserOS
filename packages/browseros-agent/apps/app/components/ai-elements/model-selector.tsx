import type { ComponentProps, ReactNode } from 'react'
import {
  Command,
  CommandDialog,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandSeparator,
  CommandShortcut,
} from '@/components/ui/command'
import {
  Dialog,
  DialogContent,
  DialogTitle,
  DialogTrigger,
} from '@/components/ui/dialog'
import { cn } from '@/lib/utils'

export type ModelSelectorProps = ComponentProps<typeof Dialog>

/**
 * @public
 */
export const ModelSelector = (props: ModelSelectorProps) => (
  <Dialog {...props} />
)

export type ModelSelectorTriggerProps = ComponentProps<typeof DialogTrigger>

/**
 * @public
 */
export const ModelSelectorTrigger = (props: ModelSelectorTriggerProps) => (
  <DialogTrigger {...props} />
)

export type ModelSelectorContentProps = ComponentProps<typeof DialogContent> & {
  title?: ReactNode
}

/**
 * @public
 */
export const ModelSelectorContent = ({
  className,
  children,
  title = 'Model Selector',
  ...props
}: ModelSelectorContentProps) => (
  <DialogContent className={cn('p-0', className)} {...props}>
    <DialogTitle className="sr-only">{title}</DialogTitle>
    <Command className="**:data-[slot=command-input-wrapper]:h-auto">
      {children}
    </Command>
  </DialogContent>
)

export type ModelSelectorDialogProps = ComponentProps<typeof CommandDialog>

/**
 * @public
 */
export const ModelSelectorDialog = (props: ModelSelectorDialogProps) => (
  <CommandDialog {...props} />
)

export type ModelSelectorInputProps = ComponentProps<typeof CommandInput>

/**
 * @public
 */
export const ModelSelectorInput = ({
  className,
  ...props
}: ModelSelectorInputProps) => (
  <CommandInput className={cn('h-auto py-3.5', className)} {...props} />
)

export type ModelSelectorListProps = ComponentProps<typeof CommandList>

/**
 * @public
 */
export const ModelSelectorList = (props: ModelSelectorListProps) => (
  <CommandList {...props} />
)

export type ModelSelectorEmptyProps = ComponentProps<typeof CommandEmpty>

/**
 * @public
 */
export const ModelSelectorEmpty = (props: ModelSelectorEmptyProps) => (
  <CommandEmpty {...props} />
)

export type ModelSelectorGroupProps = ComponentProps<typeof CommandGroup>

/**
 * @public
 */
export const ModelSelectorGroup = (props: ModelSelectorGroupProps) => (
  <CommandGroup {...props} />
)

export type ModelSelectorItemProps = ComponentProps<typeof CommandItem>

/**
 * @public
 */
export const ModelSelectorItem = (props: ModelSelectorItemProps) => (
  <CommandItem {...props} />
)

export type ModelSelectorShortcutProps = ComponentProps<typeof CommandShortcut>

/**
 * @public
 */
export const ModelSelectorShortcut = (props: ModelSelectorShortcutProps) => (
  <CommandShortcut {...props} />
)

export type ModelSelectorSeparatorProps = ComponentProps<
  typeof CommandSeparator
>

/**
 * @public
 */
export const ModelSelectorSeparator = (props: ModelSelectorSeparatorProps) => (
  <CommandSeparator {...props} />
)

export type ModelSelectorLogoProps = Omit<
  ComponentProps<'img'>,
  'src' | 'alt'
> & {
  provider:
    | 'moonshotai-cn'
    | 'lucidquery'
    | 'moonshotai'
    | 'zai-coding-plan'
    | 'alibaba'
    | 'xai'
    | 'vultr'
    | 'nvidia'
    | 'upstage'
    | 'groq'
    | 'github-copilot'
    | 'mistral'
    | 'vercel'
    | 'nebius'
    | 'deepseek'
    | 'alibaba-cn'
    | 'google-vertex-anthropic'
    | 'venice'
    | 'chutes'
    | 'cortecs'
    | 'github-models'
    | 'togetherai'
    | 'azure'
    | 'baseten'
    | 'huggingface'
    | 'opencode'
    | 'fastrouter'
    | 'google'
    | 'google-vertex'
    | 'cloudflare-workers-ai'
    | 'inception'
    | 'wandb'
    | 'openai'
    | 'zhipuai-coding-plan'
    | 'perplexity'
    | 'openrouter'
    | 'zenmux'
    | 'v0'
    | 'iflowcn'
    | 'synthetic'
    | 'deepinfra'
    | 'zhipuai'
    | 'submodel'
    | 'zai'
    | 'inference'
    | 'requesty'
    | 'morph'
    | 'lmstudio'
    | 'anthropic'
    | 'aihubmix'
    | 'fireworks-ai'
    | 'modelscope'
    | 'llama'
    | 'scaleway'
    | 'amazon-bedrock'
    | 'cerebras'
    | (string & {})
}

/**
 * @public
 */
export const ModelSelectorLogo = ({
  provider,
  className,
  ...props
}: ModelSelectorLogoProps) => (
  <img
    {...props}
    alt={`${provider} logo`}
    className={cn('size-3 dark:invert', className)}
    height={12}
    src={`https://models.dev/logos/${provider}.svg`}
    width={12}
  />
)

export type ModelSelectorLogoGroupProps = ComponentProps<'div'>

/**
 * @public
 */
export const ModelSelectorLogoGroup = ({
  className,
  ...props
}: ModelSelectorLogoGroupProps) => (
  <div
    className={cn(
      'flex shrink-0 items-center -space-x-1 [&>img]:rounded-full [&>img]:bg-background [&>img]:p-px [&>img]:ring-1 dark:[&>img]:bg-foreground',
      className,
    )}
    {...props}
  />
)

export type ModelSelectorNameProps = ComponentProps<'span'>

/**
 * @public
 */
export const ModelSelectorName = ({
  className,
  ...props
}: ModelSelectorNameProps) => (
  <span className={cn('flex-1 truncate text-left', className)} {...props} />
)
