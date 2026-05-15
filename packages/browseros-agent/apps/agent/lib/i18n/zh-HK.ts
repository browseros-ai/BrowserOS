/**
 * 繁體中文（香港）翻譯文件
 * 包含所有用戶可見的介面文字：面板、MCP、AI Chat 等
 */

export interface Translation {
  // 通用
  loading: string
  error: string
  success: string
  cancel: string
  save: string
  delete: string
  edit: string
  close: string
  back: string
  next: string
  finish: string
  skip: string
  search: string
  settings: string
  profile: string
  logout: string
  login: string
  signIn: string
  signUp: string
  
  // AI Chat - 側邊板
  chat: string
  agent: string
  chatWithThisPage: string
  agentAtYourService: string
  askQuestionsAboutCurrentPage: string
  letAIAutomateTasks: string
  summarizeThisPage: string
  whatTopicsDoesThisPageTalkAbout: string
  extractCommentsFromThisPage: string
  readAboutOurVisionAndUpvote: string
  supportBrowserOSOnGithub: string
  openAmazonAndOrder: string
  
  // Chat Input
  askAboutThisPage: string
  whatShouldIDo: string
  transcribing: string
  stopRecording: string
  voiceInput: string
  send: string
  stop: string
  attachTabs: string
  selectWorkspaceFolder: string
  connectApps: string
  
  // Chat Messages
  you: string
  assistant: string
  thinking: string
  reasoning: string
  toolUse: string
  codeExecution: string
  webNavigation: string
  copy: string
  copied: string
  like: string
  dislike: string
  liked: string
  disliked: string
  
  // MCP Settings
  mcpSettings: string
  mcpServers: string
  mcpTools: string
  serverUrl: string
  serverStatus: string
  connected: string
  disconnected: string
  connecting: string
  restartServer: string
  refreshTools: string
  addCustomServer: string
  addManagedServer: string
  serverName: string
  serverDescription: string
  apiKey: string
  apiKeyRequired: string
  configureApiKey: string
  toolsAvailable: string
  noToolsAvailable: string
  quickSetup: string
  managedServers: string
  customServers: string
  availableServers: string
  authenticated: string
  notAuthenticated: string
  authenticate: string
  removeServer: string
  editServer: string
  
  // Workflow
  workflows: string
  createWorkflow: string
  runWorkflow: string
  workflowName: string
  workflowDescription: string
  scheduledTasks: string
  newScheduledTask: string
  scheduleName: string
  scheduleFrequency: string
  scheduleNextRun: string
  scheduleLastRun: string
  scheduleResults: string
  
  // Navigation
  home: string
  memory: string
  skills: string
  soul: string
  customization: string
  usage: string
  llmHub: string
  aiSettings: string
  searchProvider: string
  onboarding: string
  
  // Auth
  signInWithEmail: string
  sendMagicLink: string
  magicLinkSent: string
  checkYourEmail: string
  logoutConfirm: string
  welcomeBack: string
  getStarted: string
  
  // Workspace
  workspace: string
  folder: string
  folders: string
  newFolder: string
  renameFolder: string
  deleteFolder: string
  selectFolder: string
  
  // Voice
  startRecording: string
  stopRecordingButton: string
  recording: string
  audioProcessing: string
  microphoneError: string
  permissionDenied: string
  
  // Tabs
  tabs: string
  currentTab: string
  allTabs: string
  attachCurrentTab: string
  removeAllTabs: string
  
  // Suggestions
  suggestions: string
  tryThese: string
  popularActions: string
  
  // Status
  ready: string
  processing: string
  streaming: string
  submitted: string
  failed: string
  
  // Onboarding
  welcomeToBrowserOS: string
  setupComplete: string
  letsGetStarted: string
  connectYourApps: string
  chooseYourProvider: string
  customizeExperience: string
  
  // Errors
  agentUrlError: string
  chatError: string
  connectionFailed: string
  pleaseTryAgain: string
  somethingWentWrong: string
  
  // Time
  justNow: string
  minutesAgo: string
  hoursAgo: string
  daysAgo: string
  today: string
  yesterday: string
  thisWeek: string
  lastWeek: string
  
  // Actions
  run: string
  execute: string
  view: string
  export: string
  import: string
  share: string
  duplicate: string
  archive: string
  restore: string
  enable: string
  disable: string
  
  // n8n Integration
  n8nWorkflows: string
  executeN8NWorkflow: string
  n8nServer: string
  automation: string
  trigger: string
  webhook: string
}

export const zhHK: Translation = {
  // 通用
  loading: '載入中...',
  error: '錯誤',
  success: '成功',
  cancel: '取消',
  save: '儲存',
  delete: '刪除',
  edit: '編輯',
  close: '關閉',
  back: '返回',
  next: '下一步',
  finish: '完成',
  skip: '跳過',
  search: '搜尋',
  settings: '設定',
  profile: '個人資料',
  logout: '登出',
  login: '登入',
  signIn: '登入',
  signUp: '註冊',
  
  // AI Chat - 側邊板
  chat: '對話',
  agent: '代理',
  chatWithThisPage: '與此頁面對話',
  agentAtYourService: '代理為您服務',
  askQuestionsAboutCurrentPage: '詢問關於當前頁面的問題或任何話題',
  letAIAutomateTasks: '讓 AI 自動化任務並為您瀏覽',
  summarizeThisPage: '總結此頁面',
  whatTopicsDoesThisPageTalkAbout: '此頁面討論什麼主題？',
  extractCommentsFromThisPage: '從此頁面提取評論',
  readAboutOurVisionAndUpvote: '閱讀我們的願景並投票支持',
  supportBrowserOSOnGithub: '在 Github 上支持 BrowserOS',
  openAmazonAndOrder: '開啟 Amazon 並訂購 Sensodyne 牙膏',
  
  // Chat Input
  askAboutThisPage: '詢問關於此頁面...',
  whatShouldIDo: '我應該做什麼？',
  transcribing: '正在轉錄...',
  stopRecording: '停止錄音',
  voiceInput: '語音輸入',
  send: '發送',
  stop: '停止',
  attachTabs: '附加分頁 (@)',
  selectWorkspaceFolder: '選擇工作區資料夾',
  connectApps: '連接應用程式',
  
  // Chat Messages
  you: '您',
  assistant: '助手',
  thinking: '思考中',
  reasoning: '推理',
  toolUse: '工具使用',
  codeExecution: '程式碼執行',
  webNavigation: '網頁導航',
  copy: '複製',
  copied: '已複製',
  like: '讚好',
  dislike: '不喜歡',
  liked: '已讚好',
  disliked: '已標記為不喜歡',
  
  // MCP Settings
  mcpSettings: 'MCP 設定',
  mcpServers: 'MCP 伺服器',
  mcpTools: 'MCP 工具',
  serverUrl: '伺服器網址',
  serverStatus: '伺服器狀態',
  connected: '已連接',
  disconnected: '未連接',
  connecting: '連接中',
  restartServer: '重新啟動伺服器',
  refreshTools: '刷新工具',
  addCustomServer: '添加自定義伺服器',
  addManagedServer: '添加託管伺服器',
  serverName: '伺服器名稱',
  serverDescription: '伺服器描述',
  apiKey: 'API 金鑰',
  apiKeyRequired: '需要 API 金鑰',
  configureApiKey: '配置 API 金鑰',
  toolsAvailable: '可用工具',
  noToolsAvailable: '暫無可用工具',
  quickSetup: '快速設定',
  managedServers: '託管伺服器',
  customServers: '自定義伺服器',
  availableServers: '可用伺服器',
  authenticated: '已認證',
  notAuthenticated: '未認證',
  authenticate: '認證',
  removeServer: '移除伺服器',
  editServer: '編輯伺服器',
  
  // Workflow
  workflows: '工作流程',
  createWorkflow: '建立工作流程',
  runWorkflow: '執行工作流程',
  workflowName: '工作流程名稱',
  workflowDescription: '工作流程描述',
  scheduledTasks: '排程任務',
  newScheduledTask: '新增排程任務',
  scheduleName: '排程名稱',
  scheduleFrequency: '排程頻率',
  scheduleNextRun: '下次執行',
  scheduleLastRun: '上次執行',
  scheduleResults: '排程結果',
  
  // Navigation
  home: '主頁',
  memory: '記憶',
  skills: '技能',
  soul: '靈魂',
  customization: '自定義',
  usage: '使用量',
  llmHub: 'LLM 中心',
  aiSettings: 'AI 設定',
  searchProvider: '搜尋提供者',
  onboarding: '新手指引',
  
  // Auth
  signInWithEmail: '使用電郵登入',
  sendMagicLink: '發送魔法連結',
  magicLinkSent: '魔法連結已發送',
  checkYourEmail: '請檢查您的電郵',
  logoutConfirm: '確定要登出嗎？',
  welcomeBack: '歡迎回來',
  getStarted: '開始使用',
  
  // Workspace
  workspace: '工作區',
  folder: '資料夾',
  folders: '資料夾',
  newFolder: '新增資料夾',
  renameFolder: '重新命名資料夾',
  deleteFolder: '刪除資料夾',
  selectFolder: '選擇資料夾',
  
  // Voice
  startRecording: '開始錄音',
  stopRecordingButton: '停止錄音',
  recording: '錄音中',
  audioProcessing: '音頻處理中',
  microphoneError: '麥克風錯誤',
  permissionDenied: '權限被拒絕',
  
  // Tabs
  tabs: '分頁',
  currentTab: '當前分頁',
  allTabs: '所有分頁',
  attachCurrentTab: '附加當前分頁',
  removeAllTabs: '移除所有分頁',
  
  // Suggestions
  suggestions: '建議',
  tryThese: '試試這些',
  popularActions: '熱門操作',
  
  // Status
  ready: '就緒',
  processing: '處理中',
  streaming: '串流中',
  submitted: '已提交',
  failed: '失敗',
  
  // Onboarding
  welcomeToBrowserOS: '歡迎來到 BrowserOS',
  setupComplete: '設定完成',
  letsGetStarted: '讓我們開始吧',
  connectYourApps: '連接您的應用程式',
  chooseYourProvider: '選擇您的提供者',
  customizeExperience: '自定義體驗',
  
  // Errors
  agentUrlError: '代理網址錯誤',
  chatError: '對話錯誤',
  connectionFailed: '連接失敗',
  pleaseTryAgain: '請再試一次',
  somethingWentWrong: '出現問題',
  
  // Time
  justNow: '剛剛',
  minutesAgo: '分鐘前',
  hoursAgo: '小時前',
  daysAgo: '天前',
  today: '今天',
  yesterday: '昨天',
  thisWeek: '本週',
  lastWeek: '上週',
  
  // Actions
  run: '執行',
  execute: '執行',
  view: '檢視',
  export: '匯出',
  import: '匯入',
  share: '分享',
  duplicate: '複製',
  archive: '歸檔',
  restore: '還原',
  enable: '啟用',
  disable: '停用',
  
  // n8n Integration
  n8nWorkflows: 'n8n 工作流程',
  executeN8NWorkflow: '執行 n8n 工作流程',
  n8nServer: 'n8n 伺服器',
  automation: '自動化',
  trigger: '觸發器',
  webhook: '網頁鉤子',
}

export default zhHK
