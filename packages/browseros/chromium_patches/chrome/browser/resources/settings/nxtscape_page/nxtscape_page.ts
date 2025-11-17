diff --git a/chrome/browser/resources/settings/blockbrowser_page/blockbrowser_page.ts b/chrome/browser/resources/settings/blockbrowser_page/blockbrowser_page.ts
new file mode 100644
index 0000000000000..80e988de4d595
--- /dev/null
+++ b/chrome/browser/resources/settings/blockbrowser_page/blockbrowser_page.ts
@@ -0,0 +1,1099 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+/**
+ * @fileoverview 'settings-blockbrowser-page' contains AI provider-specific settings.
+ */
+
+import '../settings_page/settings_section.js';
+import '../settings_page_styles.css.js';
+import '../settings_shared.css.js';
+import '../controls/settings_toggle_button.js';
+import 'chrome://resources/cr_elements/cr_button/cr_button.js';
+import 'chrome://resources/cr_elements/cr_icon/cr_icon.js';
+import 'chrome://resources/cr_elements/icons.html.js';
+import 'chrome://resources/cr_elements/cr_shared_style.css.js';
+import 'chrome://resources/cr_elements/cr_input/cr_input.js';
+import 'chrome://resources/cr_elements/cr_checkbox/cr_checkbox.js';
+
+import {PrefsMixin} from '/shared/settings/prefs/prefs_mixin.js';
+import {PolymerElement} from 'chrome://resources/polymer/v3_0/polymer/polymer_bundled.min.js';
+
+import {getTemplate} from './blockbrowser_page.html.js';
+import {MODELS_DATA, getModelsForProvider, getModelContextLength} from './models_data.js';
+
+const SettingsBlockBrowserPageElementBase = PrefsMixin(PolymerElement);
+
+export enum ProviderType {
+  BROWSEROS = 'browseros',
+  OPENAI = 'openai',
+  OPENAI_COMPATIBLE = 'openai_compatible',
+  ANTHROPIC = 'anthropic',
+  GOOGLE_GEMINI = 'google_gemini',
+  OLLAMA = 'ollama',
+  OPENROUTER = 'openrouter',
+  CUSTOM = 'custom'
+}
+
+export interface ProviderConfig {
+  id: string;
+  name: string;
+  type: ProviderType;
+  isDefault: boolean;
+  isBuiltIn?: boolean;
+  
+  baseUrl?: string;
+  apiKey?: string;
+  modelId?: string;
+  
+  capabilities?: {
+    supportsImages: boolean;
+  };
+  
+  modelConfig?: {
+    contextWindow: number;
+    temperature: number;
+  };
+  
+  createdAt: string;
+  updatedAt: string;
+}
+
+export interface ProviderTemplate {
+  name: string;
+  type: ProviderType;
+  baseUrl: string;
+  modelId: string;
+  capabilities: {
+    supportsImages: boolean;
+  };
+  modelConfig: {
+    contextWindow: number;
+    temperature: number;
+  };
+}
+
+interface AIProviderPreferences {
+  defaultProviderId: string;
+  providers: ProviderConfig[];
+}
+
+const PROVIDER_DEFAULTS: Record<string, Partial<ProviderConfig>> = {
+  [ProviderType.OPENAI]: {
+    baseUrl: 'https://api.openai.com/v1',
+    modelId: 'gpt-4.1',
+    capabilities: { supportsImages: true },
+    modelConfig: { contextWindow: 1047576, temperature: 0.7 }
+  },
+  [ProviderType.OPENAI_COMPATIBLE]: {
+    baseUrl: '',
+    modelId: 'openai/gpt-oss-20b',
+    capabilities: { supportsImages: true },
+    modelConfig: { contextWindow: 131072, temperature: 0.7 }
+  },
+  [ProviderType.ANTHROPIC]: {
+    baseUrl: 'https://api.anthropic.com',
+    modelId: 'claude-sonnet-4-20250514',
+    capabilities: { supportsImages: true },
+    modelConfig: { contextWindow: 200000, temperature: 0.7 }
+  },
+  [ProviderType.GOOGLE_GEMINI]: {
+    baseUrl: 'https://generativelanguage.googleapis.com',
+    modelId: 'gemini-2.5-flash',
+    capabilities: { supportsImages: true },
+    modelConfig: { contextWindow: 1048576, temperature: 0.7 }
+  },
+  [ProviderType.OLLAMA]: {
+    baseUrl: 'http://127.0.0.1:11434',
+    modelId: 'gpt-oss:20b',
+    capabilities: { supportsImages: false },
+    modelConfig: { contextWindow: 4096, temperature: 0.7 }
+  },
+  [ProviderType.OPENROUTER]: {
+    baseUrl: 'https://openrouter.ai/api/v1',
+    modelId: 'openai/gpt-4.1',
+    capabilities: { supportsImages: true },
+    modelConfig: { contextWindow: 1047576, temperature: 0.7 }
+  },
+  [ProviderType.CUSTOM]: {
+    baseUrl: '',
+    modelId: '',
+    capabilities: { supportsImages: false },
+    modelConfig: { contextWindow: 4096, temperature: 0.7 }
+  }
+};
+
+// Model suggestions for each provider type
+// Provider templates for quick setup
+const PROVIDER_TEMPLATES: ProviderTemplate[] = [
+  {
+    name: 'OpenAI',
+    type: ProviderType.OPENAI,
+    baseUrl: 'https://api.openai.com/v1',
+    modelId: 'gpt-4.1',
+    capabilities: { supportsImages: true },
+    modelConfig: { contextWindow: 1047576, temperature: 0.7 },
+  },
+  {
+    name: 'Claude',
+    type: ProviderType.ANTHROPIC,
+    baseUrl: 'https://api.anthropic.com',
+    modelId: 'claude-sonnet-4-20250514',
+    capabilities: { supportsImages: true },
+    modelConfig: { contextWindow: 200000, temperature: 0.7 },
+  },
+  {
+    name: 'Gemini',
+    type: ProviderType.GOOGLE_GEMINI,
+    baseUrl: 'https://generativelanguage.googleapis.com',
+    modelId: 'gemini-2.5-flash',
+    capabilities: { supportsImages: true },
+    modelConfig: { contextWindow: 1048576, temperature: 0.7 },
+  },
+  {
+    name: 'Ollama',
+    type: ProviderType.OLLAMA,
+    baseUrl: 'http://127.0.0.1:11434',
+    modelId: 'gpt-oss:20b',
+    capabilities: { supportsImages: false },
+    modelConfig: { contextWindow: 4096, temperature: 0.7 },
+  },
+  {
+    name: 'OpenRouter',
+    type: ProviderType.OPENROUTER,
+    baseUrl: 'https://openrouter.ai/api/v1',
+    modelId: 'openai/gpt-4.1',
+    capabilities: { supportsImages: true },
+    modelConfig: { contextWindow: 1047576, temperature: 0.7 },
+  },
+  {
+    name: 'LM Studio',
+    type: ProviderType.OPENAI_COMPATIBLE,
+    baseUrl: 'http://127.0.0.1:1234/v1/',
+    modelId: 'openai/gpt-oss-20b',
+    capabilities: { supportsImages: false },
+    modelConfig: { contextWindow: 131072, temperature: 0.7 },
+  },
+];
+
+// Function to get model suggestions from the real data
+function getModelSuggestions(providerType: string): string[] {
+  const models = getModelsForProvider(providerType);
+  return models.map(m => m.model_id);
+}
+
+export class SettingsBlockBrowserPageElement extends SettingsBlockBrowserPageElementBase {
+  static get is() {
+    return 'settings-blockbrowser-page';
+  }
+
+  static get template() {
+    return getTemplate();
+  }
+
+  static get properties() {
+    return {
+      prefs: {
+        type: Object,
+        notify: true,
+        observer: 'onPrefsChanged_',
+      },
+      
+      providers_: {
+        type: Array,
+        value: () => [],
+      },
+      
+      defaultProviderId_: {
+        type: String,
+        value: 'browseros',
+      },
+      
+      showProviderForm_: {
+        type: Boolean,
+        value: false,
+      },
+      
+      editingProvider_: {
+        type: Object,
+        value: null,
+      },
+      
+      dialogProviderType_: {
+        type: String,
+        value: ProviderType.OPENAI_COMPATIBLE,
+      },
+      
+      dialogProviderName_: {
+        type: String,
+        value: '',
+      },
+      
+      dialogBaseUrl_: {
+        type: String,
+        value: '',
+      },
+      
+      dialogApiKey_: {
+        type: String,
+        value: '',
+      },
+      
+      dialogModelId_: {
+        type: String,
+        value: '',
+      },
+      
+      dialogSupportsImages_: {
+        type: Boolean,
+        value: true,
+      },
+      
+      dialogContextWindow_: {
+        type: Number,
+        value: 128000,
+      },
+      
+      dialogTemperature_: {
+        type: Number,
+        value: 0.7,
+      },
+      
+      isTestingConnection_: {
+        type: Boolean,
+        value: false,
+      },
+      
+      filteredModelSuggestions_: {
+        type: Array,
+        value: () => [],
+      },
+      
+      showModelDropdown_: {
+        type: Boolean,
+        value: false,
+      },
+      
+      selectedSuggestionIndex_: {
+        type: Number,
+        value: -1,
+      },
+      
+      showTemplates_: {
+        type: Boolean,
+        value: true,
+      },
+      
+      providerTemplates_: {
+        type: Array,
+        value: () => PROVIDER_TEMPLATES,
+      },
+    };
+  }
+
+  declare prefs: any;
+  private declare providers_: ProviderConfig[];
+  private declare defaultProviderId_: string;
+  private declare showProviderForm_: boolean;
+  private declare editingProvider_: ProviderConfig | null;
+  private declare dialogProviderType_: ProviderType;
+  private declare dialogProviderName_: string;
+  private declare dialogBaseUrl_: string;
+  private declare dialogApiKey_: string;
+  private declare dialogModelId_: string;
+  private declare dialogSupportsImages_: boolean;
+  private declare dialogContextWindow_: number;
+  private declare dialogTemperature_: number;
+  private declare isTestingConnection_: boolean;
+  private declare filteredModelSuggestions_: string[];
+  private declare showModelDropdown_: boolean;
+  private declare selectedSuggestionIndex_: number;
+  private declare showTemplates_: boolean;
+  private declare providerTemplates_: ProviderTemplate[];
+
+  override ready() {
+    super.ready();
+    // Don't load providers immediately - wait for prefs to be available
+    // The onPrefsChanged_ observer will handle initial load
+  }
+
+  private onPrefsChanged_() {
+    if (this.prefs && this.prefs.browseros) {
+      this.loadProviders_();
+    }
+  }
+
+  private loadProviders_() {
+    // Load from preferences or initialize with BlockBrowser
+    if (!this.prefs || !this.prefs.browseros || !this.prefs.browseros.providers) {
+      // Prefs not ready yet, initialize defaults locally only
+      this.initializeDefaultProvidersLocally_();
+      return;
+    }
+    
+    const stored = this.getPref('browseros.providers');
+    if (stored && stored.value) {
+      try {
+        const data = JSON.parse(stored.value) as AIProviderPreferences;
+        this.providers_ = data.providers;
+        this.defaultProviderId_ = data.defaultProviderId;
+        
+        // Ensure BlockBrowser is always present
+        if (!this.providers_.some(p => p.id === 'browseros')) {
+          this.initializeDefaultProviders_();
+        }
+      } catch (e) {
+        this.initializeDefaultProviders_();
+      }
+    } else {
+      this.initializeDefaultProviders_();
+    }
+  }
+
+  private initializeDefaultProvidersLocally_() {
+    // Initialize providers locally without saving to prefs (prefs not ready)
+    const now = new Date().toISOString();
+    
+    const browseros: ProviderConfig = {
+      id: 'browseros',
+      name: 'BlockBrowser',
+      type: ProviderType.BROWSEROS,
+      isDefault: true,
+      isBuiltIn: true,
+      createdAt: now,
+      updatedAt: now,
+    };
+    
+    this.providers_ = [browseros];
+    this.defaultProviderId_ = 'browseros';
+    // Don't save yet - prefs aren't ready
+  }
+
+  private initializeDefaultProviders_() {
+    const now = new Date().toISOString();
+    
+    const browseros: ProviderConfig = {
+      id: 'browseros',
+      name: 'BlockBrowser',
+      type: ProviderType.BROWSEROS,
+      isDefault: true,
+      isBuiltIn: true,
+      createdAt: now,
+      updatedAt: now,
+    };
+    
+    this.providers_ = [browseros];
+    this.defaultProviderId_ = 'browseros';
+    
+    // Only save if prefs are ready
+    if (this.prefs && this.prefs.browseros && this.prefs.browseros.providers !== undefined) {
+      this.saveProviders_();
+    }
+  }
+
+  private saveProviders_() {
+    // Safety check: don't save if prefs aren't ready
+    if (!this.prefs || !this.prefs.browseros || this.prefs.browseros.providers === undefined) {
+      console.warn('blockbrowser: Cannot save providers - prefs not ready');
+      return;
+    }
+    
+    const data: AIProviderPreferences = {
+      defaultProviderId: this.defaultProviderId_,
+      providers: this.providers_,
+    };
+    
+    // @ts-ignore: setPrefValue exists at runtime from PrefsMixin
+    this.setPrefValue('browseros.providers', JSON.stringify(data));
+    console.log('blockbrowser: Saving providers:', data);
+  }
+
+  private generateId_(): string {
+    return 'provider_' + Date.now() + '_' + Math.random().toString(36).substr(2, 9);
+  }
+
+  private onAddProvider_() {
+    console.log('blockbrowser: Add Provider clicked');
+    
+    // Toggle the form visibility
+    this.set('showProviderForm_', !this.showProviderForm_);
+    
+    if (this.showProviderForm_) {
+      this.set('editingProvider_', null);
+      this.set('dialogProviderType_', ProviderType.OPENAI);
+      this.set('dialogProviderName_', '');
+      this.set('dialogBaseUrl_', '');
+      this.set('dialogApiKey_', '');
+      this.set('dialogModelId_', '');
+      this.set('dialogSupportsImages_', true);
+      this.set('dialogContextWindow_', 128000);
+      this.set('dialogTemperature_', 0.7);
+      
+      // Apply defaults for the selected type and update model suggestions
+      this.onProviderTypeChange_();
+      this.updateModelSuggestions_();
+      
+      // Reset dropdown state
+      this.set('showModelDropdown_', false);
+      this.set('selectedSuggestionIndex_', -1);
+      
+      // Scroll to form after a brief delay to ensure it's rendered
+      this.scrollToForm_();
+    }
+    
+    console.log('blockbrowser: Form visibility:', this.showProviderForm_);
+  }
+
+  private onEditProvider_(e: Event) {
+    const target = e.currentTarget as HTMLElement;
+    const providerId = target.dataset['providerId'];
+    const provider = this.providers_.find(p => p.id === providerId);
+    
+    if (!provider || provider.isBuiltIn) return;
+    
+    this.set('editingProvider_', provider);
+    this.set('dialogProviderType_', provider.type);
+    this.set('dialogProviderName_', provider.name);
+    this.set('dialogBaseUrl_', provider.baseUrl || '');
+    this.set('dialogApiKey_', provider.apiKey || '');
+    this.set('dialogModelId_', provider.modelId || '');
+    this.set('dialogSupportsImages_', provider.capabilities?.supportsImages || false);
+    this.set('dialogContextWindow_', provider.modelConfig?.contextWindow || 128000);
+    this.set('dialogTemperature_', provider.modelConfig?.temperature || 0.7);
+    
+    // Reset dropdown state
+    this.set('showModelDropdown_', false);
+    this.set('selectedSuggestionIndex_', -1);
+    this.updateModelSuggestions_();
+    
+    this.set('showProviderForm_', true);
+    
+    // Scroll to form after showing it
+    this.scrollToForm_();
+  }
+
+  private onDeleteProvider_(e: Event) {
+    e.stopPropagation();
+    const target = e.currentTarget as HTMLElement;
+    const providerId = target.dataset['providerId'];
+    const provider = this.providers_.find(p => p.id === providerId);
+    
+    console.log('blockbrowser: Delete provider clicked:', providerId, provider);
+    
+    // Don't allow deleting built-in providers
+    if (!provider || provider.isBuiltIn) return;
+    
+    // Delete immediately without confirmation
+    const index = this.providers_.findIndex(p => p.id === providerId);
+    if (index !== -1) {
+      this.splice('providers_', index, 1);
+      
+      // If deleted provider was default, set BlockBrowser as default
+      if (provider.id === this.defaultProviderId_) {
+        this.set('defaultProviderId_', 'browseros');
+        this.updateProvidersDefaultStatus_();
+      }
+      
+      this.saveProviders_();
+      this.showStatusMessage_('Provider deleted');
+    }
+  }
+
+  private onProviderTypeChange_(event?: Event) {
+    // Get the new value from the event if available, otherwise use the bound property
+    let providerType = this.dialogProviderType_;
+    if (event && event.target) {
+      const selectElement = event.target as HTMLSelectElement;
+      providerType = selectElement.value as ProviderType;
+      // Also update the bound property
+      this.set('dialogProviderType_', providerType);
+    }
+    
+    console.log('blockbrowser: Provider type changed to:', providerType);
+    
+    const defaults = PROVIDER_DEFAULTS[providerType];
+    if (defaults) {
+      console.log('blockbrowser: Applying defaults:', defaults);
+      // Use Polymer's set method to ensure proper data binding
+      this.set('dialogBaseUrl_', defaults.baseUrl || '');
+      this.set('dialogModelId_', defaults.modelId || '');
+      this.set('dialogSupportsImages_', defaults.capabilities?.supportsImages || false);
+      this.set('dialogContextWindow_', defaults.modelConfig?.contextWindow || 128000);
+      this.set('dialogTemperature_', defaults.modelConfig?.temperature || 0.7);
+    }
+    
+    // Update model suggestions for the new provider type
+    this.updateModelSuggestions_();
+  }
+  
+  private updateModelSuggestions_() {
+    const providerType = this.dialogProviderType_;
+    
+    // Get models from the JSON data
+    const models = getModelsForProvider(providerType);
+    const suggestions = models.map(m => m.model_id);
+    
+    // If no models from data, fall back to getModelSuggestions
+    if (suggestions.length === 0) {
+      const fallbackSuggestions = getModelSuggestions(providerType);
+      this.set('filteredModelSuggestions_', fallbackSuggestions);
+    } else {
+      this.set('filteredModelSuggestions_', suggestions);
+    }
+  }
+  
+  private onModelIdInput_(event: Event) {
+    const input = event.target as HTMLInputElement;
+    const value = input.value.toLowerCase();
+    const providerType = this.dialogProviderType_;
+    
+    // Get models from data
+    const models = getModelsForProvider(providerType);
+    let allSuggestions = models.map(m => m.model_id);
+    
+    // Fall back to function if no data
+    if (allSuggestions.length === 0) {
+      allSuggestions = getModelSuggestions(providerType);
+    }
+    
+    if (value === '') {
+      this.set('filteredModelSuggestions_', allSuggestions);
+    } else {
+      const filtered = allSuggestions.filter(model => 
+        model.toLowerCase().includes(value)
+      );
+      this.set('filteredModelSuggestions_', filtered);
+    }
+    
+    // Reset selection when filtering
+    this.set('selectedSuggestionIndex_', -1);
+    
+    // Show dropdown if there are suggestions
+    if (this.filteredModelSuggestions_.length > 0) {
+      this.set('showModelDropdown_', true);
+    }
+  }
+  
+  private onModelIdFocus_(_event: Event) {
+    // Show dropdown on focus if there are suggestions or user can add custom model
+    if (this.filteredModelSuggestions_.length > 0 || this.dialogModelId_) {
+      this.set('showModelDropdown_', true);
+    }
+  }
+  
+  private onModelIdBlur_(_event: Event) {
+    // Delay hiding to allow click events on suggestions
+    setTimeout(() => {
+      // Only hide if not clicking on a dropdown item
+      if (this.showModelDropdown_) {
+        this.set('showModelDropdown_', false);
+        this.set('selectedSuggestionIndex_', -1);
+      }
+    }, 250);
+  }
+  
+  private onModelIdKeydown_(event: KeyboardEvent) {
+    if (!this.showModelDropdown_) {
+      // Show dropdown on arrow down when it's not visible
+      if (event.key === 'ArrowDown') {
+        event.preventDefault();
+        if (this.filteredModelSuggestions_.length > 0 || this.dialogModelId_) {
+          this.set('showModelDropdown_', true);
+        }
+      }
+      return;
+    }
+    
+    const hasCustomOption = this.shouldShowCustomOption_(this.dialogModelId_, this.filteredModelSuggestions_);
+    const totalOptions = this.filteredModelSuggestions_.length + (hasCustomOption ? 1 : 0);
+    
+    if (totalOptions === 0) {
+      if (event.key === 'Escape') {
+        this.set('showModelDropdown_', false);
+      }
+      return;
+    }
+    
+    switch (event.key) {
+      case 'ArrowDown':
+        event.preventDefault();
+        const nextIndex = Math.min(
+          this.selectedSuggestionIndex_ + 1, 
+          this.filteredModelSuggestions_.length - 1
+        );
+        this.set('selectedSuggestionIndex_', nextIndex);
+        break;
+        
+      case 'ArrowUp':
+        event.preventDefault();
+        const prevIndex = Math.max(this.selectedSuggestionIndex_ - 1, hasCustomOption ? -2 : -1);
+        this.set('selectedSuggestionIndex_', prevIndex);
+        break;
+        
+      case 'Enter':
+        event.preventDefault();
+        if (this.selectedSuggestionIndex_ === -2 && hasCustomOption) {
+          // Custom option selected
+          this.onSelectCustomModel_(event);
+        } else if (this.selectedSuggestionIndex_ >= 0) {
+          const selectedModel = this.filteredModelSuggestions_[this.selectedSuggestionIndex_];
+          this.set('dialogModelId_', selectedModel);
+          
+          // Auto-set context length from model data
+          const contextLength = getModelContextLength(this.dialogProviderType_, selectedModel);
+          if (contextLength) {
+            this.set('dialogContextWindow_', contextLength);
+            console.log('blockbrowser: Auto-set context length to:', contextLength);
+          }
+          
+          this.set('showModelDropdown_', false);
+          this.set('selectedSuggestionIndex_', -1);
+        } else if (this.dialogModelId_) {
+          // Just close dropdown if no selection but there's text
+          this.set('showModelDropdown_', false);
+          this.set('selectedSuggestionIndex_', -1);
+        }
+        break;
+        
+      case 'Escape':
+        this.set('showModelDropdown_', false);
+        this.set('selectedSuggestionIndex_', -1);
+        break;
+    }
+  }
+  
+  private onSelectSuggestion_(event: Event) {
+    event.stopPropagation();
+    event.preventDefault();
+    
+    const target = event.currentTarget as HTMLElement;
+    const model = target.dataset['model'];
+    console.log('blockbrowser: Model suggestion clicked:', model);
+    
+    if (model) {
+      console.log('blockbrowser: Setting dialogModelId_ to:', model);
+      this.set('dialogModelId_', model);
+      this.set('showModelDropdown_', false);
+      this.set('selectedSuggestionIndex_', -1);
+      
+      // Auto-set context length from model data
+      const contextLength = getModelContextLength(this.dialogProviderType_, model);
+      if (contextLength) {
+        this.set('dialogContextWindow_', contextLength);
+        console.log('blockbrowser: Auto-set context length to:', contextLength);
+      }
+      
+      // Force update the input field value
+      const input = this.shadowRoot?.querySelector('.model-id-input') as HTMLInputElement;
+      if (input) {
+        input.value = model;
+        console.log('blockbrowser: Input value set to:', input.value);
+      }
+    }
+  }
+  
+  private getDropdownClass_(show: boolean): string {
+    return show ? 'show' : '';
+  }
+  
+  private getSuggestionClass_(index: number, selectedIndex: number): string {
+    return index === selectedIndex ? 'selected' : '';
+  }
+  
+  private getCustomOptionClass_(selectedIndex: number): string {
+    return selectedIndex === -2 ? 'selected' : '';
+  }
+  
+  private shouldShowCustomOption_(modelId: string, suggestions: string[]): boolean {
+    // Show custom option if user has typed something that's not empty
+    // and either no suggestions match or the exact value isn't in suggestions
+    if (!modelId || modelId.trim() === '') {
+      return false;
+    }
+    
+    // Don't show custom option if the exact model ID is already in suggestions
+    const exactMatch = suggestions.some(s => s.toLowerCase() === modelId.toLowerCase());
+    return !exactMatch;
+  }
+  
+  private shouldShowEmptyState_(modelId: string, suggestions: string[]): boolean {
+    // Show empty state only when there's no input and no suggestions
+    return (!modelId || modelId.trim() === '') && suggestions.length === 0;
+  }
+  
+  private onSelectCustomModel_(event: Event) {
+    event.stopPropagation();
+    event.preventDefault();
+    
+    // The model ID is already set, just close the dropdown
+    this.set('showModelDropdown_', false);
+    this.set('selectedSuggestionIndex_', -1);
+    
+    // Log that a custom model was selected
+    console.log('blockbrowser: Custom model selected:', this.dialogModelId_);
+    
+    // Force update the input field value to ensure it's synced
+    const input = this.shadowRoot?.querySelector('.model-id-input') as HTMLInputElement;
+    if (input && this.dialogModelId_) {
+      input.value = this.dialogModelId_;
+    }
+  }
+  
+  private onToggleDropdown_(event: Event) {
+    event.stopPropagation();
+    event.preventDefault();
+    
+    if (this.showModelDropdown_) {
+      this.set('showModelDropdown_', false);
+      this.set('selectedSuggestionIndex_', -1);
+    } else {
+      // Update suggestions first
+      this.updateModelSuggestions_();
+      // Trigger filtering if there's existing text
+      if (this.dialogModelId_) {
+        const value = this.dialogModelId_.toLowerCase();
+        const providerType = this.dialogProviderType_;
+        const allSuggestions = getModelSuggestions(providerType);
+        
+        if (value === '') {
+          this.set('filteredModelSuggestions_', allSuggestions);
+        } else {
+          const filtered = allSuggestions.filter(model => 
+            model.toLowerCase().includes(value)
+          );
+          this.set('filteredModelSuggestions_', filtered);
+        }
+        this.set('selectedSuggestionIndex_', -1);
+      }
+      this.set('showModelDropdown_', true);
+    }
+  }
+
+  private async testConnection_() {
+    this.isTestingConnection_ = true;
+    
+    // Simulate API test (in real implementation, this would call C++ backend)
+    await new Promise(resolve => setTimeout(resolve, 1500));
+    
+    this.isTestingConnection_ = false;
+    this.showStatusMessage_('Connection successful!');
+  }
+
+  private saveProvider_() {
+    if (!this.validateProviderForm_()) {
+      return;
+    }
+    
+    const now = new Date().toISOString();
+    
+    if (this.editingProvider_) {
+      // Update existing provider
+      const index = this.providers_.findIndex(p => p.id === this.editingProvider_!.id);
+      if (index !== -1) {
+        const updated: ProviderConfig = {
+          ...this.editingProvider_,
+          name: this.dialogProviderName_,
+          type: this.dialogProviderType_,
+          baseUrl: this.dialogBaseUrl_,
+          apiKey: this.dialogApiKey_,
+          modelId: this.dialogModelId_,
+          capabilities: {
+            supportsImages: this.dialogSupportsImages_,
+          },
+          modelConfig: {
+            contextWindow: this.dialogContextWindow_,
+            temperature: this.dialogTemperature_,
+          },
+          updatedAt: now,
+        };
+        
+        this.set(`providers_.${index}`, updated);
+      }
+    } else {
+      // Add new provider
+      const newProvider: ProviderConfig = {
+        id: this.generateId_(),
+        name: this.dialogProviderName_,
+        type: this.dialogProviderType_,
+        isDefault: false,
+        isBuiltIn: false,
+        baseUrl: this.dialogBaseUrl_,
+        apiKey: this.dialogApiKey_,
+        modelId: this.dialogModelId_,
+        capabilities: {
+          supportsImages: this.dialogSupportsImages_,
+        },
+        modelConfig: {
+          contextWindow: this.dialogContextWindow_,
+          temperature: this.dialogTemperature_,
+        },
+        createdAt: now,
+        updatedAt: now,
+      };
+      this.push('providers_', newProvider);
+      
+      chrome.send('logBlockBrowserMetric', ['settings.provider.added', {
+        provider_type: newProvider.type,
+        model_id: newProvider.modelId
+      }]);
+    }
+    
+    this.saveProviders_();
+    this.showStatusMessage_(this.editingProvider_ ? 'Provider updated' : 'Provider added');
+    this.closeProviderForm_();
+  }
+
+  private validateProviderForm_(): boolean {
+    console.log('blockbrowser: Validating form:', {
+      name: this.dialogProviderName_,
+      type: this.dialogProviderType_,
+      apiKey: this.dialogApiKey_,
+      modelId: this.dialogModelId_
+    });
+    
+    if (!this.dialogProviderName_ || !this.dialogProviderName_.trim()) {
+      this.showStatusMessage_('Provider name is required', true);
+      return false;
+    }
+    
+    // API key is optional for all providers, but show warning if missing for certain types
+    const apiKeyOptionalTypes = [ProviderType.OLLAMA, ProviderType.OPENAI_COMPATIBLE, ProviderType.CUSTOM];
+    if (!this.dialogApiKey_ || !this.dialogApiKey_.trim()) {
+      if (!apiKeyOptionalTypes.includes(this.dialogProviderType_)) {
+        // Show warning but don't block
+        this.showStatusMessage_('Warning: API key not provided. Provider may not work without it.', false);
+      }
+    }
+    
+    if (!this.dialogModelId_ || !this.dialogModelId_.trim()) {
+      this.showStatusMessage_('Model ID is required', true);
+      return false;
+    }
+    
+    return true;
+  }
+
+  private closeProviderForm_() {
+    this.set('showProviderForm_', false);
+    this.editingProvider_ = null;
+  }
+  
+  private scrollToForm_() {
+    // Use requestAnimationFrame to ensure DOM is updated
+    requestAnimationFrame(() => {
+      const formCard = this.shadowRoot?.querySelector('.provider-form-card') as HTMLElement;
+      if (formCard) {
+        // Get the position of the form relative to the viewport
+        const rect = formCard.getBoundingClientRect();
+        
+        // Always scroll to form when opening from templates (which are at bottom)
+        // or if form is not fully visible
+        if (rect.top < 0 || rect.bottom > window.innerHeight || rect.top > window.innerHeight / 2) {
+          // Smooth scroll to the form with some offset from top
+          formCard.scrollIntoView({ 
+            behavior: 'smooth', 
+            block: 'start'
+          });
+          
+          // Add a visual highlight effect
+          formCard.classList.add('highlight');
+          
+          // Add a pulse animation
+          formCard.style.animation = 'pulse 1s ease-out';
+          
+          // Remove effects after animation
+          setTimeout(() => {
+            if (formCard) {
+              formCard.style.animation = '';
+              formCard.classList.remove('highlight');
+            }
+          }, 2000);
+        }
+        
+        // Focus on the first input field for better UX
+        // Delay to allow scroll animation to complete
+        setTimeout(() => {
+          const firstInput = formCard.querySelector('input:not([type="checkbox"])') as HTMLInputElement;
+          if (firstInput) {
+            firstInput.focus();
+            firstInput.select(); // Select text if present for easy replacement
+          }
+        }, 500);
+      }
+    });
+  }
+
+  private onDefaultProviderChange_(e: Event) {
+    const select = e.target as HTMLSelectElement;
+    const oldProviderId = this.defaultProviderId_;
+    this.defaultProviderId_ = select.value;
+    this.updateProvidersDefaultStatus_();
+    this.saveProviders_();
+    this.showStatusMessage_('Default provider changed');
+
+    chrome.send('logBlockBrowserMetric', ['settings.default_provider.changed', {
+      old_provider_id: oldProviderId,
+      new_provider_id: this.defaultProviderId_
+    }]);
+  }
+
+  private updateProvidersDefaultStatus_() {
+    this.providers_ = this.providers_.map(p => ({
+      ...p,
+      isDefault: p.id === this.defaultProviderId_
+    }));
+  }
+
+  private setAsDefault_(e: Event) {
+    const target = e.currentTarget as HTMLElement;
+    const providerId = target.dataset['providerId'];
+    const oldProviderId = this.defaultProviderId_;
+    this.defaultProviderId_ = providerId!;
+    this.updateProvidersDefaultStatus_();
+    this.saveProviders_();
+    this.showStatusMessage_('Default provider changed');
+
+    chrome.send('logBlockBrowserMetric', ['settings.default_provider.changed', {
+      old_provider_id: oldProviderId,
+      new_provider_id: this.defaultProviderId_
+    }]);
+  }
+
+  private getProviderIcon_(type: ProviderType): string {
+    const icons: Record<ProviderType, string> = {
+      [ProviderType.BROWSEROS]: 'B',
+      [ProviderType.OPENAI]: 'O',
+      [ProviderType.OPENAI_COMPATIBLE]: 'O',
+      [ProviderType.ANTHROPIC]: 'A',
+      [ProviderType.GOOGLE_GEMINI]: 'G',
+      [ProviderType.OLLAMA]: 'L',
+      [ProviderType.OPENROUTER]: 'R',
+      [ProviderType.CUSTOM]: 'C',
+    };
+    return icons[type] || 'AI';
+  }
+
+  private getProviderCardClass_(provider: ProviderConfig): string {
+    let classes = 'provider-card';
+    if (provider.isDefault) {
+      classes += ' selected';
+    }
+    if (provider.type === ProviderType.BROWSEROS) {
+      classes += ' browseros';
+    }
+    return classes;
+  }
+  
+  private shouldShowRadio_(provider: ProviderConfig): boolean {
+    return !provider.isBuiltIn;
+  }
+  
+  private shouldShowActions_(provider: ProviderConfig): boolean {
+    return !provider.isBuiltIn;
+  }
+  
+  private onToggleTemplates_() {
+    this.set('showTemplates_', !this.showTemplates_);
+  }
+  
+  private onUseTemplate_(event: Event) {
+    const target = event.currentTarget as HTMLElement;
+    const templateIndex = parseInt(target.dataset['templateIndex'] || '0', 10);
+    const template = this.providerTemplates_[templateIndex];
+    
+    if (!template) return;
+    
+    // Show the form
+    this.set('showProviderForm_', true);
+    this.set('editingProvider_', null);
+    
+    // Populate form with template values
+    this.set('dialogProviderType_', template.type);
+    this.set('dialogProviderName_', template.name);
+    this.set('dialogBaseUrl_', template.baseUrl);
+    this.set('dialogModelId_', template.modelId);
+    this.set('dialogApiKey_', ''); // User needs to provide this
+    this.set('dialogSupportsImages_', template.capabilities.supportsImages);
+    this.set('dialogContextWindow_', template.modelConfig.contextWindow);
+    this.set('dialogTemperature_', template.modelConfig.temperature);
+    
+    // Update model suggestions (but don't call onProviderTypeChange_ as it would overwrite template values)
+    this.updateModelSuggestions_();
+    
+    // Reset dropdown state
+    this.set('showModelDropdown_', false);
+    this.set('selectedSuggestionIndex_', -1);
+    
+    // Scroll to form and focus API key field
+    this.scrollToForm_();
+    setTimeout(() => {
+      const apiKeyInput = this.shadowRoot?.querySelector('input[type="password"]') as HTMLInputElement;
+      if (apiKeyInput) {
+        apiKeyInput.focus();
+      }
+    }, 600);
+    
+    console.log('blockbrowser: Using template:', template.name);
+  }
+
+  private getProviderSubtitle_(provider: ProviderConfig): string {
+    if (provider.type === ProviderType.BROWSEROS) {
+      return 'Automatically chooses the best model for each task';
+    }
+    
+    const parts = [];
+    if (provider.modelId) {
+      parts.push(`Model: ${provider.modelId}`);
+    }
+    if (provider.baseUrl && !this.isDefaultUrl_(provider.type, provider.baseUrl)) {
+      parts.push(`URL: ${this.truncateUrl_(provider.baseUrl)}`);
+    }
+    return parts.join(' • ') || 'Not configured';
+  }
+
+  private isDefaultUrl_(type: ProviderType, url: string): boolean {
+    const defaults = PROVIDER_DEFAULTS[type];
+    return defaults?.baseUrl === url;
+  }
+
+  private truncateUrl_(url: string): string {
+    if (url.length > 30) {
+      return url.substring(0, 27) + '...';
+    }
+    return url;
+  }
+
+  private getFormCardClass_(show: boolean): string {
+    return show ? 'show' : '';
+  }
+  
+  private getExpandIconClass_(expanded: boolean): string {
+    return expanded ? 'expanded' : '';
+  }
+
+  private showStatusMessage_(message?: string, isError: boolean = false) {
+    if (!this.shadowRoot) return;
+    
+    const statusMessage = this.shadowRoot.querySelector('#statusMessage') as HTMLElement;
+    if (statusMessage) {
+      if (message) {
+        statusMessage.textContent = message;
+      }
+      statusMessage.classList.toggle('error', isError);
+      statusMessage.classList.add('show');
+      setTimeout(() => {
+        statusMessage.classList.remove('show');
+      }, 2000);
+    }
+  }
+}
+
+declare global {
+  interface HTMLElementTagNameMap {
+    'settings-blockbrowser-page': SettingsBlockBrowserPageElement;
+  }
+}
+
+customElements.define(
+    SettingsBlockBrowserPageElement.is, SettingsBlockBrowserPageElement);
