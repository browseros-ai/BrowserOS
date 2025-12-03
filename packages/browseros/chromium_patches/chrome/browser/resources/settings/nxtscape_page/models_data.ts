diff --git a/chrome/browser/resources/settings/nxtscape_page/models_data.ts b/chrome/browser/resources/settings/nxtscape_page/models_data.ts
new file mode 100644
index 0000000000000..36638c1910a09
--- /dev/null
+++ b/chrome/browser/resources/settings/nxtscape_page/models_data.ts
@@ -0,0 +1,279 @@
+// Model data interface
+export interface ModelInfo {
+  model_id: string;
+  context_length: number;
+  requires_computer_use?: boolean;
+}
+
+export interface ModelsData {
+  openai?: ModelInfo[];
+  claude?: ModelInfo[];
+  anthropic?: ModelInfo[];
+  gemini?: ModelInfo[];
+  google_gemini?: ModelInfo[];
+  openrouter?: ModelInfo[];
+  ollama?: ModelInfo[];
+  lmstudio?: ModelInfo[];
+}
+
+// Direct export of models data
+export const MODELS_DATA: ModelsData = {
+  openai: [
+    { model_id: 'gpt-5-nano', context_length: 400000 },
+    { model_id: 'gpt-5', context_length: 400000 },
+    { model_id: 'gpt-5-mini', context_length: 400000 },
+    { model_id: 'o1-mini', context_length: 128000 },
+    { model_id: 'o1', context_length: 200000 },
+    { model_id: 'o3-mini', context_length: 200000 },
+    { model_id: 'o1-pro', context_length: 200000 },
+    { model_id: 'o3', context_length: 200000 },
+    { model_id: 'o4-mini', context_length: 200000 },
+    { model_id: 'gpt-4.1', context_length: 1047576 },
+    { model_id: 'gpt-4.1-mini', context_length: 1047576 },
+    { model_id: 'gpt-4.1-nano', context_length: 1047576 },
+    { model_id: 'o3-pro', context_length: 200000 },
+  ],
+  claude: [
+    { model_id: 'claude-opus-4-1-20250805', context_length: 200000 },
+    { model_id: 'claude-opus-4-20250514', context_length: 200000 },
+    { model_id: 'claude-sonnet-4-20250514', context_length: 200000 },
+    { model_id: 'claude-3-7-sonnet-20250219', context_length: 200000 },
+    { model_id: 'claude-3-5-sonnet-20241022', context_length: 200000 },
+    { model_id: 'claude-3-5-haiku-20241022', context_length: 200000 },
+    { model_id: 'claude-3-5-sonnet-20240620', context_length: 200000 },
+    { model_id: 'claude-3-haiku-20240307', context_length: 200000 },
+    { model_id: 'claude-3-opus-20240229', context_length: 200000 },
+  ],
+  gemini: [
+    { model_id: 'gemini-1.5-pro-latest', context_length: 2000000 },
+    { model_id: 'gemini-1.5-pro-002', context_length: 2000000 },
+    { model_id: 'gemini-1.5-pro', context_length: 2000000 },
+    { model_id: 'gemini-1.5-flash-latest', context_length: 1000000 },
+    { model_id: 'gemini-1.5-flash', context_length: 1000000 },
+    { model_id: 'gemini-1.5-flash-002', context_length: 1000000 },
+    { model_id: 'gemini-1.5-flash-8b', context_length: 1000000 },
+    { model_id: 'gemini-1.5-flash-8b-001', context_length: 1000000 },
+    { model_id: 'gemini-1.5-flash-8b-latest', context_length: 1000000 },
+    { model_id: 'gemini-2.5-pro-preview-03-25', context_length: 1048576 },
+    { model_id: 'gemini-2.5-flash-preview-05-20', context_length: 1048576 },
+    { model_id: 'gemini-2.5-flash', context_length: 1048576 },
+    { model_id: 'gemini-2.5-flash-lite-preview-06-17', context_length: 1048576 },
+    { model_id: 'gemini-2.5-pro-preview-05-06', context_length: 1048576 },
+    { model_id: 'gemini-2.5-pro-preview-06-05', context_length: 1048576 },
+    { model_id: 'gemini-2.5-pro', context_length: 1048576 },
+    { model_id: 'gemini-2.5-computer-use-preview-10-2025', context_length: 1048576, requires_computer_use: true },
+    { model_id: 'gemini-2.0-flash-exp', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash-001', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash-exp-image-generation', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash-lite-001', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash-lite', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash-preview-image-generation', context_length: 32768 },
+    { model_id: 'gemini-2.0-flash-lite-preview-02-05', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash-lite-preview', context_length: 1048576 },
+    { model_id: 'gemini-2.0-pro-exp', context_length: 1048576 },
+    { model_id: 'gemini-2.0-pro-exp-02-05', context_length: 1048576 },
+    { model_id: 'gemini-exp-1206', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash-thinking-exp-01-21', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash-thinking-exp', context_length: 1048576 },
+    { model_id: 'gemini-2.0-flash-thinking-exp-1219', context_length: 1048576 },
+    { model_id: 'gemini-2.5-flash-preview-tts', context_length: 8192 },
+    { model_id: 'gemini-2.5-pro-preview-tts', context_length: 8192 },
+    { model_id: 'learnlm-2.0-flash-experimental', context_length: 1048576 },
+    { model_id: 'gemma-3-1b-it', context_length: 32768 },
+    { model_id: 'gemma-3-4b-it', context_length: 131072 },
+    { model_id: 'gemma-3-12b-it', context_length: 96000 },
+    { model_id: 'gemma-3-27b-it', context_length: 96000 },
+    { model_id: 'gemma-3n-e4b-it', context_length: 32768 },
+    { model_id: 'gemma-3n-e2b-it', context_length: 8192 },
+    { model_id: 'gemini-2.5-flash-lite', context_length: 1048576 },
+  ],
+  openrouter: [
+    { model_id: 'deepseek/deepseek-chat-v3.1', context_length: 163840 },
+    { model_id: 'deepseek/deepseek-v3.1-base', context_length: 163840 },
+    { model_id: 'mistralai/mistral-medium-3.1', context_length: 262144 },
+    { model_id: 'baidu/ernie-4.5-21b-a3b', context_length: 120000 },
+    { model_id: 'baidu/ernie-4.5-vl-28b-a3b', context_length: 30000 },
+    { model_id: 'z-ai/glm-4.5v', context_length: 65536 },
+    { model_id: 'ai21/jamba-mini-1.7', context_length: 256000 },
+    { model_id: 'ai21/jamba-large-1.7', context_length: 256000 },
+    { model_id: 'openai/gpt-5-chat', context_length: 400000 },
+    { model_id: 'openai/gpt-5', context_length: 400000 },
+    { model_id: 'openai/gpt-5-mini', context_length: 400000 },
+    { model_id: 'openai/gpt-5-nano', context_length: 400000 },
+    { model_id: 'openai/gpt-oss-120b', context_length: 131000 },
+    { model_id: 'openai/gpt-oss-20b:free', context_length: 131072 },
+    { model_id: 'openai/gpt-oss-20b', context_length: 131000 },
+    { model_id: 'anthropic/claude-opus-4.1', context_length: 200000 },
+    { model_id: 'anthropic/claude-opus-4', context_length: 200000 },
+    { model_id: 'anthropic/claude-sonnet-4', context_length: 200000 },
+    { model_id: 'anthropic/claude-3.7-sonnet', context_length: 200000 },
+    { model_id: 'anthropic/claude-3.7-sonnet:thinking', context_length: 200000 },
+    { model_id: 'anthropic/claude-3.5-haiku-20241022', context_length: 200000 },
+    { model_id: 'anthropic/claude-3.5-haiku', context_length: 200000 },
+    { model_id: 'anthropic/claude-3.5-sonnet', context_length: 200000 },
+    { model_id: 'mistralai/codestral-2508', context_length: 256000 },
+    { model_id: 'mistralai/codestral-2501', context_length: 262144 },
+    { model_id: 'mistralai/mistral-large-2411', context_length: 131072 },
+    { model_id: 'mistralai/mistral-large-2407', context_length: 131072 },
+    { model_id: 'mistralai/pixtral-large-2411', context_length: 131072 },
+    { model_id: 'mistralai/pixtral-12b', context_length: 32768 },
+    { model_id: 'google/gemini-2.5-flash-lite', context_length: 1048576 },
+    { model_id: 'google/gemini-2.5-flash', context_length: 1048576 },
+    { model_id: 'google/gemini-2.5-pro', context_length: 1048576 },
+    { model_id: 'google/gemini-2.0-flash-001', context_length: 1048576 },
+    { model_id: 'google/gemini-2.0-flash-exp:free', context_length: 1048576 },
+    { model_id: 'google/gemini-flash-1.5-8b', context_length: 1000000 },
+    { model_id: 'qwen/qwq-32b-preview', context_length: 32768 },
+    { model_id: 'qwen/qwq-32b:free', context_length: 32768 },
+    { model_id: 'qwen/qwq-32b', context_length: 131072 },
+    { model_id: 'qwen/qwen-2.5-coder-32b-instruct:free', context_length: 32768 },
+    { model_id: 'qwen/qwen-2.5-coder-32b-instruct', context_length: 32768 },
+    { model_id: 'qwen/qwen-2.5-72b-instruct:free', context_length: 32768 },
+    { model_id: 'qwen/qwen-2.5-72b-instruct', context_length: 32768 },
+    { model_id: 'qwen/qwen-2.5-7b-instruct', context_length: 65536 },
+    { model_id: 'meta-llama/llama-3.3-70b-instruct:free', context_length: 65536 },
+    { model_id: 'meta-llama/llama-3.3-70b-instruct', context_length: 131072 },
+    { model_id: 'meta-llama/llama-3.2-90b-vision-instruct', context_length: 131072 },
+    { model_id: 'meta-llama/llama-3.2-11b-vision-instruct:free', context_length: 131072 },
+    { model_id: 'meta-llama/llama-3.2-11b-vision-instruct', context_length: 131072 },
+    { model_id: 'meta-llama/llama-3.2-3b-instruct:free', context_length: 131072 },
+    { model_id: 'meta-llama/llama-3.2-3b-instruct', context_length: 20000 },
+    { model_id: 'meta-llama/llama-3.2-1b-instruct', context_length: 131072 },
+    { model_id: 'deepseek/deepseek-r1:free', context_length: 163840 },
+    { model_id: 'deepseek/deepseek-r1', context_length: 163840 },
+    { model_id: 'deepseek/deepseek-chat', context_length: 163840 },
+    { model_id: 'x-ai/grok-2-vision-1212', context_length: 32768 },
+    { model_id: 'x-ai/grok-2-1212', context_length: 131072 },
+    { model_id: 'openai/o1-mini', context_length: 128000 },
+    { model_id: 'openai/o1', context_length: 200000 },
+    { model_id: 'openai/o1-pro', context_length: 200000 },
+    { model_id: 'openai/o3-mini', context_length: 200000 },
+    { model_id: 'openai/o3', context_length: 200000 },
+    { model_id: 'openai/gpt-4o-2024-11-20', context_length: 128000 },
+    { model_id: 'openai/gpt-4o-mini-search-preview', context_length: 128000 },
+    { model_id: 'openai/gpt-4o-search-preview', context_length: 128000 },
+  ],
+  lmstudio: [
+    // OpenAI GPT-OSS
+    { model_id: 'openai/gpt-oss-120b', context_length: 131072 },
+    { model_id: 'openai/gpt-oss-20b', context_length: 131072 },
+    
+    // Qwen3 Models
+    { model_id: 'qwen/qwen3-4b-thinking-2507', context_length: 32768 },
+    { model_id: 'qwen/qwen3-4b-2507', context_length: 32768 },
+    { model_id: 'qwen/qwen3-coder-30b', context_length: 32768 },
+    { model_id: 'qwen/qwen3-30b-a3b-2507', context_length: 32768 },
+    { model_id: 'qwen/qwen3-coder-480b', context_length: 32768 },
+    { model_id: 'qwen/qwen3-235b-a22b-2507', context_length: 32768 },
+    { model_id: 'qwen/qwen3-235b-a22b', context_length: 32768 },
+    { model_id: 'qwen/qwen3-32b', context_length: 32768 },
+    { model_id: 'qwen/qwen3-30b-a3b', context_length: 32768 },
+    { model_id: 'qwen/qwen3-1.7b', context_length: 32768 },
+    { model_id: 'qwen/qwen3-4b', context_length: 32768 },
+    { model_id: 'qwen/qwen3-14b', context_length: 32768 },
+    { model_id: 'qwen/qwen3-8b', context_length: 32768 },
+    
+    // Mistral Models
+    { model_id: 'mistralai/devstral-small-2507', context_length: 128000 },
+    { model_id: 'mistralai/mistral-small-3.2', context_length: 128000 },
+    { model_id: 'mistralai/magistral-small', context_length: 128000 },
+    { model_id: 'mistralai/devstral-small-2505', context_length: 128000 },
+    { model_id: 'mistralai/mistral-nemo-instruct-2407', context_length: 128000 },
+    { model_id: 'mistralai/mathstral-7b-v0.1', context_length: 32768 },
+    { model_id: 'mistralai/codestral-22b-v0.1', context_length: 32768 },
+    { model_id: 'mistralai/mistral-7b-instruct-v0.3', context_length: 32768 },
+    
+    
+    // Google Gemma Models
+    { model_id: 'google/gemma-3n-e4b', context_length: 8192 },
+    { model_id: 'google/gemma-3-27b', context_length: 8192 },
+    { model_id: 'google/gemma-3-12b', context_length: 8192 },
+    { model_id: 'google/gemma-3-4b', context_length: 8192 },
+    { model_id: 'google/gemma-3-1b', context_length: 8192 },
+    
+    // DeepSeek Models
+    { model_id: 'deepseek/deepseek-r1-0528-qwen3-8b', context_length: 128000 },
+    { model_id: 'lmstudio-community/deepseek-r1-distill-llama-8b', context_length: 131072 },
+    { model_id: 'lmstudio-community/deepseek-r1-distill-qwen-7b', context_length: 131072 },
+    
+    // Microsoft Phi Models
+    { model_id: 'microsoft/phi-4-mini-reasoning', context_length: 16384 },
+    { model_id: 'microsoft/phi-4-reasoning-plus', context_length: 16384 },
+    { model_id: 'microsoft/phi-4', context_length: 16384 },
+    
+    // Qwen2.5 Models
+    { model_id: 'qwen/qwen2.5-vl-7b', context_length: 32768 },
+    { model_id: 'qwen/qwen2.5-coder-14b', context_length: 128000 },
+    { model_id: 'qwen/qwen2.5-coder-32b', context_length: 128000 },
+    
+    // Meta Llama
+    { model_id: 'meta/llama-3.3-70b', context_length: 131072 },
+  ],
+  ollama: [
+    // OpenAI gpt-oss
+    { model_id: 'gpt-oss:20b', context_length: 4096 },
+    { model_id: 'gpt-oss:120b', context_length: 4096 },
+
+    // DeepSeek R1
+    { model_id: 'deepseek-r1:7b', context_length: 4096 },
+    { model_id: 'deepseek-r1:8b', context_length: 4096 },
+    { model_id: 'deepseek-r1:14b', context_length: 4096 },
+    { model_id: 'deepseek-r1:32b', context_length: 4096 },
+    { model_id: 'deepseek-r1:70b', context_length: 4096 },
+    { model_id: 'deepseek-r1:671b', context_length: 4096 },
+
+    // Qwen3
+    { model_id: 'qwen3:0.6b', context_length: 4096 },
+    { model_id: 'qwen3:1.7b', context_length: 4096 },
+    { model_id: 'qwen3:4b', context_length: 4096 },
+    { model_id: 'qwen3:8b', context_length: 4096 },
+    { model_id: 'qwen3:14b', context_length: 4096 },
+    { model_id: 'qwen3:30b', context_length: 4096 },
+    { model_id: 'qwen3:32b', context_length: 4096 },
+    { model_id: 'qwen3:235b', context_length: 4096 },
+
+    // Llama 3.x
+    { model_id: 'llama3.1:8b', context_length: 4096 },
+    { model_id: 'llama3.1:70b', context_length: 4096 },
+    { model_id: 'llama3.1:405b', context_length: 4096 },
+    { model_id: 'llama3.2:1b', context_length: 4096 },
+    { model_id: 'llama3.2:3b', context_length: 4096 },
+    { model_id: 'llama3.3:70b', context_length: 4096 },
+
+    // Qwen2.5 (dense + coder)
+    { model_id: 'qwen2.5:7b', context_length: 4096 },
+    { model_id: 'qwen2.5:14b', context_length: 4096 },
+    { model_id: 'qwen2.5:32b', context_length: 4096 },
+    { model_id: 'qwen2.5:72b', context_length: 4096 },
+    { model_id: 'qwen2.5-coder:7b', context_length: 4096 },
+    { model_id: 'qwen2.5-coder:14b', context_length: 4096 },
+    { model_id: 'qwen2.5-coder:32b', context_length: 4096 },
+  ],
+};
+
+// Helper function to get models for a provider type
+export function getModelsForProvider(providerType: string): ModelInfo[] {
+  switch (providerType) {
+    case 'openai':
+      return MODELS_DATA.openai || [];
+    case 'anthropic':
+      return MODELS_DATA.claude || [];
+    case 'google_gemini':
+      return MODELS_DATA.gemini || [];
+    case 'openrouter':
+      return MODELS_DATA.openrouter || [];
+    case 'openai_compatible':
+      // For LM Studio and other OpenAI compatible providers
+      return MODELS_DATA.lmstudio || [];
+    case 'ollama':
+      return MODELS_DATA.ollama || [];
+    default:
+      return [];
+  }
+}
+
+// Helper to get context length for a specific model
+export function getModelContextLength(providerType: string, modelId: string): number | undefined {
+  const models = getModelsForProvider(providerType);
+  const model = models.find(m => m.model_id === modelId);
+  return model?.context_length;
+}
