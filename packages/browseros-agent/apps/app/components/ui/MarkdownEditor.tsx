import '@mdxeditor/editor/style.css'
import {
  BlockTypeSelect,
  BoldItalicUnderlineToggles,
  CreateLink,
  headingsPlugin,
  InsertThematicBreak,
  ListsToggle,
  linkDialogPlugin,
  linkPlugin,
  listsPlugin,
  MDXEditor,
  type MDXEditorMethods,
  markdownShortcutPlugin,
  quotePlugin,
  Separator,
  thematicBreakPlugin,
  toolbarPlugin,
  UndoRedo,
} from '@mdxeditor/editor'
import { Check, Copy } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { cn } from '@/lib/utils'

interface MarkdownEditorProps {
  value: string
  onChange: (value: string) => void
  placeholder?: string
  className?: string
  autoFocus?: boolean
  readOnly?: boolean
  onKeyDown?: React.KeyboardEventHandler<HTMLDivElement>
  id?: string
}

function EditorToolbar() {
  return (
    <>
      <UndoRedo />
      <Separator />
      <BoldItalicUnderlineToggles />
      <Separator />
      <BlockTypeSelect />
      <Separator />
      <ListsToggle />
      <Separator />
      <CreateLink />
      <InsertThematicBreak />
    </>
  )
}

function usePlugins(readOnly?: boolean) {
  return useMemo(
    () => [
      headingsPlugin(),
      listsPlugin(),
      quotePlugin(),
      thematicBreakPlugin(),
      linkPlugin(),
      linkDialogPlugin(),
      markdownShortcutPlugin(),
      ...(readOnly ? [] : [toolbarPlugin({ toolbarContents: EditorToolbar })]),
    ],
    [readOnly],
  )
}

function CopyMarkdownButton({
  editorRef,
}: {
  editorRef: React.RefObject<MDXEditorMethods | null>
}) {
  const [copied, setCopied] = useState(false)

  const handleCopy = async () => {
    const markdown = editorRef.current?.getMarkdown() ?? ''
    try {
      await navigator.clipboard.writeText(markdown)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      // clipboard access denied
    }
  }

  return (
    <button
      type="button"
      onClick={handleCopy}
      className="mdx-copy-button"
      title="Copy raw markdown"
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      <span>{copied ? 'Copied' : 'Copy markdown'}</span>
    </button>
  )
}

export const MarkdownEditor = ({
  value,
  onChange,
  placeholder,
  className,
  autoFocus,
  readOnly,
  onKeyDown,
  id,
}: MarkdownEditorProps) => {
  const editorRef = useRef<MDXEditorMethods>(null)
  const plugins = usePlugins(readOnly)

  const lastSetMarkdown = useRef(value)

  useEffect(() => {
    if (lastSetMarkdown.current === value) return
    const current = editorRef.current?.getMarkdown() ?? ''
    if (current !== value) {
      lastSetMarkdown.current = value
      editorRef.current?.setMarkdown(value)
    }
  }, [value])

  const handlePaste = (e: React.ClipboardEvent<HTMLDivElement>) => {
    const text = e.clipboardData.getData('text/plain')
    if (!text) return

    const markdownPattern =
      /^#{1,6}\s|^\s*[-*+]\s|^\s*\d+\.\s|\*\*.+\*\*|__.+__|\[.+\]\(.+\)|^>/m
    if (!markdownPattern.test(text)) return

    e.preventDefault()
    e.stopPropagation()
    editorRef.current?.insertMarkdown(text)
  }

  return (
    <div id={id} className={cn('mdx-editor-outer', className)}>
      {/* biome-ignore lint/a11y/noStaticElementInteractions: onKeyDown/onPasteCapture forwarding */}
      <div
        className="mdx-editor-themed"
        onKeyDown={onKeyDown}
        onPasteCapture={handlePaste}
      >
        <MDXEditor
          ref={editorRef}
          markdown={value}
          onChange={onChange}
          placeholder={placeholder}
          plugins={plugins}
          autoFocus={autoFocus}
          readOnly={readOnly}
          contentEditableClassName="mdx-content-editable prose prose-sm max-w-none dark:prose-invert"
        />
      </div>
      <div className="mdx-copy-bar">
        <CopyMarkdownButton editorRef={editorRef} />
      </div>
    </div>
  )
}
