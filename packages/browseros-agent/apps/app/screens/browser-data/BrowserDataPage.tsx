import { Bookmark, Clock3, Download, ExternalLink, Eye, Search, Trash2 } from 'lucide-react'
import { type FC, useCallback, useEffect, useMemo, useState } from 'react'
import { useNavigate, useParams } from 'react-router'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'

type DataKind = 'bookmarks' | 'history' | 'downloads'
type DataItem = {
  id: string | number
  title?: string
  url?: string
  filename?: string
  dateAdded?: number
  lastVisitTime?: number
  startTime?: string
  state?: string
  path?: string
}

const config = {
  bookmarks: { title: 'Bookmarks', subtitle: 'Pages you saved for later.', icon: Bookmark },
  history: { title: 'History', subtitle: 'Recently visited pages on this device.', icon: Clock3 },
  downloads: { title: 'Downloads', subtitle: 'Files downloaded through Request Browser.', icon: Download },
} as const

export const BrowserDataPage: FC = () => {
  const params = useParams()
  const navigate = useNavigate()
  const kind: DataKind = params.kind === 'history' || params.kind === 'downloads' ? params.kind : 'bookmarks'
  const [items, setItems] = useState<DataItem[]>([])
  const [query, setQuery] = useState('')
  const [busy, setBusy] = useState(false)

  const refresh = useCallback(async () => {
    if (kind === 'bookmarks') setItems(await chrome.bookmarks.search({}))
    else if (kind === 'history') setItems(await chrome.history.search({ text: '', startTime: 0, maxResults: 500 }))
    else setItems(await chrome.downloads.search({}))
  }, [kind])

  useEffect(() => { void refresh() }, [refresh])

  const visibleItems = useMemo(() => {
    const term = query.trim().toLowerCase()
    return items.filter((item) => !term || `${item.title ?? ''} ${item.url ?? ''} ${item.filename ?? ''}`.toLowerCase().includes(term))
  }, [items, query])
  const page = config[kind]
  const Icon = page.icon

  const removeBookmark = async (id: string | number) => {
    await chrome.bookmarks.remove(String(id))
    await refresh()
  }

  const clearHistory = async () => {
    setBusy(true)
    try {
      await chrome.history.deleteAll()
      await refresh()
    } finally {
      setBusy(false)
    }
  }

  const clearDownloads = async () => {
    setBusy(true)
    try {
      await chrome.downloads.erase({})
      await refresh()
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div className="flex items-start gap-3">
          <div className="rounded-lg border bg-card p-2.5"><Icon className="size-5 text-primary" /></div>
          <div><h1 className="font-semibold text-2xl tracking-tight">{page.title}</h1><p className="mt-1 text-muted-foreground text-sm">{page.subtitle}</p></div>
        </div>
        <div className="flex items-center gap-2">
          {kind === 'history' ? <Button variant="outline" onClick={() => void clearHistory()} disabled={busy}><Trash2 className="mr-2 size-4" /> Clear history</Button> : null}
          {kind === 'downloads' ? <Button variant="outline" onClick={() => void clearDownloads()} disabled={busy}><Trash2 className="mr-2 size-4" /> Clear list</Button> : null}
          <Button variant="outline" onClick={() => void refresh()} disabled={busy}>Refresh</Button>
        </div>
      </div>
      <div className="relative"><Search className="absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" /><Input value={query} onChange={(event) => setQuery(event.target.value)} className="pl-9" placeholder={`Search ${page.title.toLowerCase()}…`} /></div>
      <div className="overflow-hidden rounded-xl border bg-card">
        {visibleItems.length === 0 ? <div className="p-12 text-center text-muted-foreground text-sm">No {page.title.toLowerCase()} yet.</div> : visibleItems.map((item) => (
          <div key={item.id} className="flex items-center gap-3 border-b px-4 py-3 last:border-b-0 hover:bg-muted/40">
            <Icon className="size-4 shrink-0 text-muted-foreground" />
            <button type="button" className="min-w-0 flex-1 text-left" onClick={() => {
              if (kind === 'downloads') void chrome.downloads.open(Number(item.id))
              else if (item.url) void chrome.tabs.create({ url: item.url })
            }}>
              <div className="truncate font-medium text-sm">{item.title || item.filename || item.url || 'Untitled'}</div>
              <div className="mt-0.5 truncate text-muted-foreground text-xs">{item.url || item.filename || item.state}</div>
            </button>
            {kind === 'downloads' ? <Button variant="ghost" size="icon" title="Show in folder" onClick={() => void chrome.downloads.show(Number(item.id))}><Eye className="size-4" /></Button> : item.url ? <ExternalLink className="size-4 text-muted-foreground" /> : null}
            {kind === 'bookmarks' ? <Button variant="ghost" size="icon" onClick={() => void removeBookmark(item.id)}><Trash2 className="size-4" /></Button> : null}
          </div>
        ))}
      </div>
      <Button variant="ghost" onClick={() => navigate('/home')}>Back to Home</Button>
    </div>
  )
}
