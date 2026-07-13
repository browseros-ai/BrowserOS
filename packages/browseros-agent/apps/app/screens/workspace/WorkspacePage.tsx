import {
  Archive,
  Database,
  FileImage,
  FolderPlus,
  Plus,
  RefreshCw,
  Save,
  Sparkles,
  Copy,
  Pencil,
  Trash2,
  Upload,
  X,
} from 'lucide-react'
import { type FC, useCallback, useEffect, useMemo, useState } from 'react'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from '@/components/ui/card'
import { Input } from '@/components/ui/input'
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs'
import { Textarea } from '@/components/ui/textarea'
import { cn } from '@/lib/utils'
import { useAgentServerUrl } from '@/modules/browseros/agent-server-url.hooks'

type Collection = {
  id: string
  name: string
  description: string | null
}

type Field = {
  id: string
  name: string
  key: string
  type: string
  position: number
}

type WorkspaceDatabase = {
  id: string
  collectionId: string | null
  name: string
  description: string | null
  fields?: Field[]
}

type WorkspaceRecord = {
  id: string
  title: string | null
  data: Record<string, unknown>
}

type Asset = {
  id: string
  filename: string
  mimeType: string
  byteSize: number
  contentHash: string
  recordId: string | null
}

type Session = {
  id: string
  goal: string
  status: string
  recap: {
    plan: {
      total: number
      completed: number
      blocked: number
      remaining: number
    }
    nextActions: string[]
    activityCount: number
  } | null
  plan: Array<{ id: string; title: string; status: string }>
  events: Array<{ id: string; kind: string; title: string; detail: string | null }>
}

async function request<T>(
  baseUrl: string,
  path: string,
  init?: RequestInit,
): Promise<T> {
  const response = await fetch(`${baseUrl}${path}`, {
    ...init,
    headers: {
      'Content-Type': 'application/json',
      ...init?.headers,
    },
  })
  const body = (await response.json().catch(() => ({}))) as T & {
    error?: string
  }
  if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`)
  return body
}

function prettyValue(value: unknown): string {
  if (value === null || value === undefined) return '—'
  if (typeof value === 'object') return JSON.stringify(value)
  return String(value)
}

function formatBytes(value: number): string {
  if (value < 1024) return `${value} B`
  if (value < 1024 * 1024) return `${Math.round(value / 1024)} KB`
  return `${(value / (1024 * 1024)).toFixed(1)} MB`
}

export const WorkspacePage: FC = () => {
  const { baseUrl, isLoading: serverLoading } = useAgentServerUrl()
  const [collections, setCollections] = useState<Collection[]>([])
  const [databases, setDatabases] = useState<WorkspaceDatabase[]>([])
  const [database, setDatabase] = useState<WorkspaceDatabase | null>(null)
  const [records, setRecords] = useState<WorkspaceRecord[]>([])
  const [assets, setAssets] = useState<Asset[]>([])
  const [sessions, setSessions] = useState<Session[]>([])
  const [session, setSession] = useState<Session | null>(null)
  const [suggestion, setSuggestion] = useState<{ message: string; basedOn: string } | null>(null)
  const [selectedCollectionId, setSelectedCollectionId] = useState<string>()
  const [selectedDatabaseId, setSelectedDatabaseId] = useState<string>()
  const [selectedRecordId, setSelectedRecordId] = useState<string>()
  const [selectedSessionId, setSelectedSessionId] = useState<string>()
  const [activeTab, setActiveTab] = useState('databases')
  const [newCollectionName, setNewCollectionName] = useState('')
  const [newDatabaseName, setNewDatabaseName] = useState('')
  const [newFieldName, setNewFieldName] = useState('')
  const [newFieldType, setNewFieldType] = useState('text')
  const [recordJson, setRecordJson] = useState('{\n  "name": ""\n}')
  const [newGoal, setNewGoal] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const selectedCollection = useMemo(
    () => collections.find((collection) => collection.id === selectedCollectionId),
    [collections, selectedCollectionId],
  )

  const loadCollections = useCallback(async () => {
    if (!baseUrl) return
    const response = await request<{ collections: Collection[] }>(baseUrl, '/workspace/collections')
    setCollections(response.collections)
    setSelectedCollectionId((current) =>
      current && response.collections.some((item) => item.id === current)
        ? current
        : response.collections[0]?.id,
    )
  }, [baseUrl])

  const loadDatabases = useCallback(async () => {
    if (!baseUrl) return
    const query = selectedCollectionId
      ? `?collectionId=${encodeURIComponent(selectedCollectionId)}`
      : ''
    const response = await request<{ databases: WorkspaceDatabase[] }>(
      baseUrl,
      `/workspace/databases${query}`,
    )
    setDatabases(response.databases)
    setSelectedDatabaseId((current) =>
      current && response.databases.some((item) => item.id === current)
        ? current
        : response.databases[0]?.id,
    )
  }, [baseUrl, selectedCollectionId])

  const loadDatabase = useCallback(async () => {
    if (!baseUrl || !selectedDatabaseId) {
      setDatabase(null)
      setRecords([])
      setAssets([])
      return
    }
    const [databaseResponse, recordsResponse] = await Promise.all([
      request<{ database: WorkspaceDatabase }>(baseUrl, `/workspace/databases/${selectedDatabaseId}`),
      request<{ records: WorkspaceRecord[] }>(
        baseUrl,
        `/workspace/databases/${selectedDatabaseId}/records`,
      ),
    ])
    setDatabase(databaseResponse.database)
    setRecords(recordsResponse.records)
    setSelectedRecordId((current) =>
      current && recordsResponse.records.some((item) => item.id === current)
        ? current
        : undefined,
    )
  }, [baseUrl, selectedDatabaseId])

  const loadAssets = useCallback(async () => {
    if (!baseUrl || !selectedRecordId) {
      setAssets([])
      return
    }
    const response = await request<{ assets: Asset[] }>(
      baseUrl,
      `/workspace/assets?recordId=${encodeURIComponent(selectedRecordId)}`,
    )
    setAssets(response.assets)
  }, [baseUrl, selectedRecordId])

  const loadSessions = useCallback(async () => {
    if (!baseUrl) return
    const response = await request<{ sessions: Session[] }>(baseUrl, '/workspace/sessions')
    setSessions(response.sessions)
    setSelectedSessionId((current) =>
      current && response.sessions.some((item) => item.id === current)
        ? current
        : response.sessions[0]?.id,
    )
  }, [baseUrl])

  const loadSession = useCallback(async () => {
    if (!baseUrl || !selectedSessionId) {
      setSession(null)
      return
    }
    const response = await request<{ session: Session }>(
      baseUrl,
      `/workspace/sessions/${selectedSessionId}`,
    )
    setSession(response.session)
  }, [baseUrl, selectedSessionId])

  const refresh = useCallback(async () => {
    if (!baseUrl) return
    setBusy(true)
    setError(null)
    try {
      await Promise.all([loadCollections(), loadDatabases(), loadSessions()])
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      setBusy(false)
    }
  }, [baseUrl, loadCollections, loadSessions])

  useEffect(() => {
    void refresh()
  }, [refresh])

  useEffect(() => {
    void loadDatabases().catch((cause) => setError(String(cause)))
  }, [loadDatabases])

  useEffect(() => {
    void loadDatabase().catch((cause) => setError(String(cause)))
  }, [loadDatabase])

  useEffect(() => {
    void loadAssets().catch((cause) => setError(String(cause)))
  }, [loadAssets])

  useEffect(() => {
    const selected = records.find((record) => record.id === selectedRecordId)
    if (selected) setRecordJson(JSON.stringify(selected.data, null, 2))
  }, [records, selectedRecordId])

  useEffect(() => {
    setSuggestion(null)
    void loadSession().catch((cause) => setError(String(cause)))
  }, [loadSession])

  const runMutation = async (action: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await action()
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      setBusy(false)
    }
  }

  const createCollection = () =>
    runMutation(async () => {
      const name = newCollectionName.trim()
      if (!baseUrl || !name) return
      await request(baseUrl, '/workspace/collections', {
        method: 'POST',
        body: JSON.stringify({ name }),
      })
      setNewCollectionName('')
      await loadCollections()
    })

  const renameCollection = () =>
    runMutation(async () => {
      if (!baseUrl || !selectedCollectionId) return
      const current = collections.find((item) => item.id === selectedCollectionId)
      if (!current) return
      const name = window.prompt('Collection name', current.name)?.trim()
      if (!name || name === current.name) return
      await request(baseUrl, `/workspace/collections/${selectedCollectionId}`, {
        method: 'PATCH',
        body: JSON.stringify({ name }),
      })
      await loadCollections()
    })

  const deleteCollection = () =>
    runMutation(async () => {
      if (!baseUrl || !selectedCollectionId) return
      const current = collections.find((item) => item.id === selectedCollectionId)
      if (!current || !window.confirm(`Delete collection "${current.name}"?`)) return
      await request(baseUrl, `/workspace/collections/${selectedCollectionId}`, { method: 'DELETE' })
      setSelectedCollectionId(undefined)
      await loadCollections()
      await loadDatabases()
    })

  const createDatabase = () =>
    runMutation(async () => {
      const name = newDatabaseName.trim()
      if (!baseUrl || !name) return
      await request(baseUrl, '/workspace/databases', {
        method: 'POST',
        body: JSON.stringify({ name, collectionId: selectedCollectionId }),
      })
      setNewDatabaseName('')
      await loadDatabases()
    })

  const renameDatabase = () =>
    runMutation(async () => {
      if (!baseUrl || !selectedDatabaseId || !database) return
      const name = window.prompt('Database name', database.name)?.trim()
      if (!name || name === database.name) return
      await request(baseUrl, `/workspace/databases/${selectedDatabaseId}`, {
        method: 'PATCH',
        body: JSON.stringify({ name }),
      })
      await loadDatabases()
      await loadDatabase()
    })

  const deleteDatabase = () =>
    runMutation(async () => {
      if (!baseUrl || !selectedDatabaseId) return
      if (!window.confirm('Delete this database and its records?')) return
      await request(baseUrl, `/workspace/databases/${selectedDatabaseId}`, {
        method: 'DELETE',
      })
      setSelectedDatabaseId(undefined)
      await loadDatabases()
    })

  const createField = () =>
    runMutation(async () => {
      if (!baseUrl || !selectedDatabaseId || !newFieldName.trim()) return
      await request(baseUrl, `/workspace/databases/${selectedDatabaseId}/fields`, {
        method: 'POST',
        body: JSON.stringify({
          name: newFieldName.trim(),
          type: newFieldType,
          position: database?.fields?.length ?? 0,
        }),
      })
      setNewFieldName('')
      await loadDatabase()
    })

  const deleteField = (fieldId: string) =>
    runMutation(async () => {
      if (!baseUrl || !window.confirm('Remove this field from the database?')) return
      await request(baseUrl, `/workspace/fields/${fieldId}`, { method: 'DELETE' })
      await loadDatabase()
    })

  const saveRecord = () =>
    runMutation(async () => {
      if (!baseUrl || !selectedDatabaseId) return
      const data = JSON.parse(recordJson) as Record<string, unknown>
      await request(
        baseUrl,
        selectedRecordId
          ? `/workspace/records/${selectedRecordId}`
          : `/workspace/databases/${selectedDatabaseId}/records`,
        {
          method: selectedRecordId ? 'PATCH' : 'POST',
          body: JSON.stringify({ data }),
        },
      )
      await loadDatabase()
    })

  const deleteRecord = () =>
    runMutation(async () => {
      if (!baseUrl || !selectedRecordId || !window.confirm('Delete this record and its attached assets?')) return
      await request(baseUrl, `/workspace/records/${selectedRecordId}`, { method: 'DELETE' })
      setSelectedRecordId(undefined)
      setRecordJson('{\n  "name": ""\n}')
      await loadDatabase()
    })

  const uploadAsset = (file: File) =>
    runMutation(async () => {
      if (!baseUrl) return
      const bytes = new Uint8Array(await file.arrayBuffer())
      let binary = ''
      for (const byte of bytes) binary += String.fromCharCode(byte)
      await request(baseUrl, '/workspace/assets', {
        method: 'POST',
        body: JSON.stringify({
          filename: file.name,
          mimeType: file.type || 'application/octet-stream',
          dataBase64: btoa(binary),
          recordId: selectedRecordId,
        }),
      })
      await loadAssets()
    })

  const deleteAsset = (assetId: string) =>
    runMutation(async () => {
      if (!baseUrl || !window.confirm('Delete this asset?')) return
      await request(baseUrl, `/workspace/assets/${assetId}`, { method: 'DELETE' })
      await loadAssets()
    })

  const createSession = () =>
    runMutation(async () => {
      if (!baseUrl || !newGoal.trim()) return
      await request(baseUrl, '/workspace/sessions', {
        method: 'POST',
        body: JSON.stringify({
          goal: newGoal.trim(),
          collectionId: selectedCollectionId,
          databaseId: selectedDatabaseId,
          plan: [
            {
              title: 'Review the goal and choose authorized sources',
              toolCategory: 'browser',
            },
            {
              title: 'Extract, verify, and save results',
              toolCategory: 'extraction',
            },
          ],
        }),
      })
      setNewGoal('')
      await loadSessions()
    })

  const generateRecap = () =>
    runMutation(async () => {
      if (!baseUrl || !selectedSessionId) return
      await request(baseUrl, `/workspace/sessions/${selectedSessionId}/recap`, {
        method: 'POST',
      })
      await loadSession()
    })

  const suggestNextMessage = () =>
    runMutation(async () => {
      if (!baseUrl || !selectedSessionId) return
      const response = await request<{ suggestion: { message: string; basedOn: string } }>(baseUrl, `/workspace/sessions/${selectedSessionId}/suggestion`, {
        method: 'POST',
      })
      setSuggestion(response.suggestion)
    })

  if (serverLoading) {
    return <WorkspaceEmptyState title="Connecting to Request Browser" detail="Opening your local workspace database…" />
  }

  return (
    <div className="fade-in slide-in-from-bottom-3 animate-in space-y-6 duration-500">
      <header className="flex items-start justify-between gap-4">
        <div>
          <div className="mb-2 flex items-center gap-2 text-muted-foreground text-xs uppercase tracking-[0.18em]">
            <Database className="size-3.5" />
            Workspace
          </div>
          <h1 className="font-semibold text-3xl tracking-tight">Research, organized.</h1>
          <p className="mt-2 max-w-2xl text-muted-foreground text-sm leading-6">
            Keep structured records, source links, files, and goal recaps together. Every saved item keeps its trail back to the page it came from.
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => void refresh()} disabled={busy}>
          <RefreshCw className={cn('mr-2 size-4', busy && 'animate-spin')} />
          Refresh
        </Button>
      </header>

      {error ? (
        <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-4 py-3 text-destructive text-sm">
          {error}
        </div>
      ) : null}

      <Tabs value={activeTab} onValueChange={setActiveTab}>
        <TabsList>
          <TabsTrigger value="databases">
            <Database className="mr-2 size-4" /> Databases
          </TabsTrigger>
          <TabsTrigger value="sessions">
            <Sparkles className="mr-2 size-4" /> Research sessions
          </TabsTrigger>
        </TabsList>

        <TabsContent value="databases" className="mt-5">
          <div className="grid gap-5 lg:grid-cols-[220px_minmax(0,1fr)]">
            <Card className="h-fit border-border/70 bg-card/70">
              <CardHeader className="gap-3 border-b px-4 py-4">
                <div className="flex items-center justify-between">
                  <CardTitle className="text-sm">Collections</CardTitle>
                  <FolderPlus className="size-4 text-muted-foreground" />
                </div>
                <div className="flex gap-2">
                  <Input
                    value={newCollectionName}
                    onChange={(event) => setNewCollectionName(event.target.value)}
                    placeholder="New collection"
                    className="h-8 text-xs"
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') void createCollection()
                    }}
                  />
                  <Button size="icon" className="size-8 shrink-0" onClick={() => void createCollection()}>
                    <Plus className="size-4" />
                  </Button>
                </div>
              </CardHeader>
              <CardContent className="space-y-1 p-2">
                {collections.length === 0 ? (
                  <p className="px-2 py-4 text-muted-foreground text-xs">Create a collection to give your research a home.</p>
                ) : (
                  collections.map((collection) => (
                    <div key={collection.id} className="group flex items-center gap-1">
                      <button
                        type="button"
                        onClick={() => setSelectedCollectionId(collection.id)}
                        className={cn(
                          'flex min-w-0 flex-1 items-center gap-2 rounded-md px-2.5 py-2 text-left text-sm transition-colors hover:bg-muted',
                          selectedCollectionId === collection.id && 'bg-muted font-medium',
                        )}
                      >
                        <Archive className="size-3.5 shrink-0 text-muted-foreground" />
                        <span className="truncate">{collection.name}</span>
                      </button>
                      {selectedCollectionId === collection.id ? (
                        <div className="flex shrink-0">
                          <Button variant="ghost" size="icon" className="size-7" title="Rename collection" onClick={() => void renameCollection()}>
                            <Pencil className="size-3" />
                          </Button>
                          <Button variant="ghost" size="icon" className="size-7 text-destructive hover:text-destructive" title="Delete collection" onClick={() => void deleteCollection()}>
                            <Trash2 className="size-3" />
                          </Button>
                        </div>
                      ) : null}
                    </div>
                  ))
                )}
              </CardContent>
            </Card>

            <Card className="min-w-0 border-border/70 bg-card/70">
              <CardHeader className="gap-3 border-b px-5 py-4">
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div>
                    <CardTitle className="text-base">{selectedCollection?.name ?? 'All databases'}</CardTitle>
                    <CardDescription className="mt-1">Choose a database to manage fields, records, and source assets.</CardDescription>
                  </div>
                  {database ? (
                    <div className="flex gap-1">
                      <Button variant="ghost" size="sm" onClick={() => void renameDatabase()}>
                        <Pencil className="mr-2 size-4" /> Rename
                      </Button>
                      <Button variant="ghost" size="sm" className="text-destructive hover:text-destructive" onClick={() => void deleteDatabase()}>
                        <Trash2 className="mr-2 size-4" /> Delete database
                      </Button>
                    </div>
                  ) : null}
                </div>
                <div className="flex flex-wrap gap-2">
                  <select
                    value={selectedDatabaseId ?? ''}
                    onChange={(event) => setSelectedDatabaseId(event.target.value || undefined)}
                    className="h-9 min-w-52 rounded-md border bg-background px-3 text-sm"
                  >
                    <option value="">Select a database…</option>
                    {databases.map((item) => (
                      <option key={item.id} value={item.id}>{item.name}</option>
                    ))}
                  </select>
                  <Input
                    value={newDatabaseName}
                    onChange={(event) => setNewDatabaseName(event.target.value)}
                    placeholder="New database"
                    className="h-9 max-w-52"
                    onKeyDown={(event) => {
                      if (event.key === 'Enter') void createDatabase()
                    }}
                  />
                  <Button variant="outline" size="sm" onClick={() => void createDatabase()}>
                    <Plus className="mr-2 size-4" /> Create
                  </Button>
                </div>
              </CardHeader>

              {!database ? (
                <CardContent className="flex min-h-64 items-center justify-center p-8">
                  <WorkspaceEmptyState title="Your database is empty" detail="Create one above, then add the fields your research needs." compact />
                </CardContent>
              ) : (
                <CardContent className="space-y-5 p-5">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-muted-foreground text-xs uppercase tracking-[0.14em]">Fields</span>
                    {(database.fields ?? []).map((field) => (
                      <Badge key={field.id} variant="outline" className="gap-1 font-normal">
                        <span>{field.name} <span className="ml-1 text-muted-foreground">{field.type}</span></span>
                        <button type="button" className="rounded-sm text-muted-foreground hover:text-destructive" title={`Remove ${field.name}`} onClick={() => void deleteField(field.id)}>
                          <X className="size-3" />
                        </button>
                      </Badge>
                    ))}
                    <div className="ml-auto flex gap-2">
                      <Input value={newFieldName} onChange={(event) => setNewFieldName(event.target.value)} placeholder="Field name" className="h-8 w-32 text-xs" />
                      <select value={newFieldType} onChange={(event) => setNewFieldType(event.target.value)} className="h-8 rounded-md border bg-background px-2 text-xs">
                        {['text', 'number', 'currency', 'date', 'url', 'asset'].map((type) => <option key={type} value={type}>{type}</option>)}
                      </select>
                      <Button size="sm" className="h-8" onClick={() => void createField()}><Plus className="mr-1.5 size-3.5" /> Add field</Button>
                    </div>
                  </div>

                  <div className="grid gap-5 xl:grid-cols-[minmax(0,1fr)_260px]">
                    <div className="min-w-0 overflow-hidden rounded-lg border">
                      <div className="flex items-center justify-between border-b bg-muted/30 px-4 py-3">
                        <div>
                          <p className="font-medium text-sm">Records</p>
                          <p className="text-muted-foreground text-xs">{records.length} saved in this database</p>
                        </div>
                        <div className="flex gap-2">
                          {selectedRecordId ? <Button variant="ghost" size="sm" onClick={() => { setSelectedRecordId(undefined); setRecordJson('{\n  "name": ""\n}') }}><Plus className="mr-2 size-4" /> New record</Button> : null}
                          <Button variant="outline" size="sm" onClick={() => void saveRecord()} disabled={busy}>
                            <Save className="mr-2 size-4" /> {selectedRecordId ? 'Update record' : 'Save JSON record'}
                          </Button>
                          {selectedRecordId ? <Button variant="ghost" size="icon" className="text-destructive hover:text-destructive" title="Delete selected record" onClick={() => void deleteRecord()} disabled={busy}><Trash2 className="size-4" /></Button> : null}
                        </div>
                      </div>
                      <div className="border-b p-3">
                        <Textarea value={recordJson} onChange={(event) => setRecordJson(event.target.value)} className="min-h-20 font-mono text-xs" aria-label="New record JSON" />
                      </div>
                      <div className="overflow-x-auto">
                        {records.length === 0 ? (
                          <p className="px-4 py-10 text-center text-muted-foreground text-sm">No records yet. Save the first extracted item as JSON.</p>
                        ) : (
                          <table className="w-full text-sm">
                            <thead className="border-b bg-muted/20 text-left text-muted-foreground text-xs">
                              <tr>
                                <th className="px-4 py-3 font-medium">Record</th>
                                {(database.fields ?? []).slice(0, 5).map((field) => <th key={field.id} className="px-4 py-3 font-medium">{field.name}</th>)}
                              </tr>
                            </thead>
                            <tbody>
                              {records.map((record) => (
                                <tr key={record.id} onClick={() => { setSelectedRecordId(record.id); setRecordJson(JSON.stringify(record.data, null, 2)) }} className={cn('cursor-pointer border-b last:border-0 hover:bg-muted/30', selectedRecordId === record.id && 'bg-muted/40')}>
                                  <td className="max-w-48 truncate px-4 py-3 font-medium">{record.title ?? record.data.name?.toString() ?? record.id.slice(0, 8)}</td>
                                  {(database.fields ?? []).slice(0, 5).map((field) => <td key={field.id} className="max-w-48 truncate px-4 py-3 text-muted-foreground">{prettyValue(record.data[field.key])}</td>)}
                                </tr>
                              ))}
                            </tbody>
                          </table>
                        )}
                      </div>
                    </div>

                    <Card className="h-fit border-border/70 bg-muted/10">
                      <CardHeader className="gap-2 px-4 py-4">
                        <CardTitle className="text-sm">Assets</CardTitle>
                        <CardDescription className="text-xs">Attach screenshots, documents, or images to the selected record.</CardDescription>
                      </CardHeader>
                      <CardContent className="space-y-3 px-4 pb-4">
                        <label className="flex cursor-pointer items-center justify-center gap-2 rounded-md border border-dashed px-3 py-4 text-muted-foreground text-xs transition-colors hover:border-primary hover:text-foreground">
                          <Upload className="size-4" /> Upload asset
                          <input type="file" className="sr-only" onChange={(event) => { const file = event.target.files?.[0]; if (file) void uploadAsset(file); event.target.value = '' }} />
                        </label>
                        {selectedRecordId ? assets.map((asset) => (
                          <div key={asset.id} className="flex items-center gap-2 rounded-md border bg-background px-2.5 py-2 text-xs">
                            <FileImage className="size-4 shrink-0 text-muted-foreground" />
                            <div className="min-w-0 flex-1"><p className="truncate font-medium">{asset.filename}</p><p className="text-muted-foreground">{formatBytes(asset.byteSize)}</p></div>
                            {baseUrl ? <a href={`${baseUrl}/workspace/assets/${asset.id}/content`} target="_blank" rel="noreferrer" className="text-primary hover:underline">Open</a> : null}
                            <Button variant="ghost" size="icon" className="size-6 text-muted-foreground hover:text-destructive" title="Delete asset" onClick={() => void deleteAsset(asset.id)}><X className="size-3.5" /></Button>
                          </div>
                        )) : <p className="text-muted-foreground text-xs">Select a record to see its assets.</p>}
                      </CardContent>
                    </Card>
                  </div>
                </CardContent>
              )}
            </Card>
          </div>
        </TabsContent>

        <TabsContent value="sessions" className="mt-5">
          <div className="grid gap-5 lg:grid-cols-[260px_minmax(0,1fr)]">
            <Card className="h-fit border-border/70 bg-card/70">
              <CardHeader className="gap-3 border-b px-4 py-4">
                <CardTitle className="text-sm">Goal threads</CardTitle>
                <Input value={newGoal} onChange={(event) => setNewGoal(event.target.value)} placeholder="What should be researched?" className="text-xs" />
                <Button size="sm" onClick={() => void createSession()} disabled={!newGoal.trim() || busy}><Plus className="mr-2 size-4" /> New research</Button>
              </CardHeader>
              <CardContent className="space-y-1 p-2">
                {sessions.length === 0 ? <p className="px-2 py-4 text-muted-foreground text-xs">Completed and active research sessions will appear here.</p> : sessions.map((item) => (
                  <button type="button" key={item.id} onClick={() => setSelectedSessionId(item.id)} className={cn('w-full rounded-md px-2.5 py-2 text-left hover:bg-muted', selectedSessionId === item.id && 'bg-muted')}>
                    <p className="truncate font-medium text-sm">{item.goal}</p>
                    <div className="mt-1 flex items-center gap-2"><Badge variant="outline" className="text-[10px]">{item.status}</Badge>{item.recap ? <span className="text-[10px] text-muted-foreground">{item.recap.activityCount} events</span> : null}</div>
                  </button>
                ))}
              </CardContent>
            </Card>
            <Card className="border-border/70 bg-card/70">
              {!session ? <CardContent className="flex min-h-64 items-center justify-center p-8"><WorkspaceEmptyState title="No research selected" detail="Create a goal thread to track a plan and its next action." compact /></CardContent> : <>
                <CardHeader className="gap-2 border-b px-5 py-5"><div className="flex flex-wrap items-start justify-between gap-3"><div><CardTitle className="text-base">{session.goal}</CardTitle><CardDescription className="mt-1">Live-safe activity summaries, plan state, and a durable recap.</CardDescription></div><Button variant="outline" size="sm" onClick={() => void generateRecap()}><Sparkles className="mr-2 size-4" /> Generate recap</Button></div></CardHeader>
                <CardContent className="grid gap-5 p-5 xl:grid-cols-[minmax(0,1fr)_260px]">
                  <div className="space-y-5"><div><div className="mb-2 flex items-center justify-between"><p className="font-medium text-sm">Plan</p><Badge variant="outline">{session.status}</Badge></div><div className="space-y-2">{session.plan.map((step, index) => <div key={step.id} className="flex items-center gap-3 rounded-md border px-3 py-2 text-sm"><span className="font-mono text-muted-foreground text-xs">{String(index + 1).padStart(2, '0')}</span><span className="flex-1">{step.title}</span><Badge variant={step.status === 'completed' ? 'default' : 'outline'}>{step.status}</Badge></div>)}</div></div><div><p className="mb-2 font-medium text-sm">Activity</p><div className="space-y-2">{session.events.length === 0 ? <p className="text-muted-foreground text-xs">No visible activity has been recorded yet.</p> : session.events.slice(-8).map((event) => <div key={event.id} className="border-muted border-l-2 px-3 py-1.5"><p className="text-sm">{event.title}</p>{event.detail ? <p className="text-muted-foreground text-xs">{event.detail}</p> : null}</div>)}</div></div></div>
                  <div className="rounded-lg border bg-muted/20 p-4"><div className="mb-3 flex items-center justify-between gap-2"><p className="font-medium text-sm">Session recap</p><Button variant="ghost" size="icon" className="size-7" title="Suggest next message" onClick={() => void suggestNextMessage()}><Sparkles className="size-3.5" /></Button></div>{session.recap ? <><div className="grid grid-cols-2 gap-2 text-xs"><RecapMetric label="Done" value={session.recap.plan.completed} /><RecapMetric label="Blocked" value={session.recap.plan.blocked} /><RecapMetric label="Remaining" value={session.recap.plan.remaining} /><RecapMetric label="Events" value={session.recap.activityCount} /></div><div className="mt-4"><p className="text-muted-foreground text-xs uppercase tracking-wide">Next actions</p>{session.recap.nextActions.length ? <ul className="mt-2 space-y-2 text-sm">{session.recap.nextActions.map((action) => <li key={action} className="flex gap-2"><span className="text-primary">→</span>{action}</li>)}</ul> : <p className="mt-2 text-muted-foreground text-sm">Nothing queued.</p>}</div></> : <p className="text-muted-foreground text-xs leading-5">Generate a recap after the agent has recorded plan progress.</p>}{suggestion ? <div className="mt-4 rounded-md border bg-background p-3"><div className="flex items-start gap-2"><p className="flex-1 text-sm leading-5">{suggestion.message}</p><Button variant="ghost" size="icon" className="size-7 shrink-0" title="Copy suggestion" onClick={() => void navigator.clipboard.writeText(suggestion.message)}><Copy className="size-3.5" /></Button></div><p className="mt-2 text-[10px] text-muted-foreground uppercase tracking-wide">Suggested next message</p></div> : null}</div>
                </CardContent>
              </>}
            </Card>
          </div>
        </TabsContent>
      </Tabs>
    </div>
  )
}

const RecapMetric: FC<{ label: string; value: number }> = ({ label, value }) => (
  <div className="rounded-md border bg-background/70 px-2.5 py-2"><p className="font-mono text-lg">{value}</p><p className="text-[10px] text-muted-foreground uppercase tracking-wide">{label}</p></div>
)

const WorkspaceEmptyState: FC<{ title: string; detail: string; compact?: boolean }> = ({ title, detail, compact }) => (
  <div className={cn('max-w-sm text-center', compact ? 'space-y-1' : 'space-y-2')}><div className="mx-auto flex size-10 items-center justify-center rounded-full border bg-muted/50"><Database className="size-4 text-muted-foreground" /></div><p className="font-medium text-sm">{title}</p><p className="text-muted-foreground text-xs leading-5">{detail}</p></div>
)
