diff --git a/chrome/browser/resources/settings/ghost_mode_page/ghost_mode_page.ts b/chrome/browser/resources/settings/ghost_mode_page/ghost_mode_page.ts
new file mode 100644
index 0000000000000..b2c3d4e5f6a7b
--- /dev/null
+++ b/chrome/browser/resources/settings/ghost_mode_page/ghost_mode_page.ts
@@ -0,0 +1,298 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+/**
+ * @fileoverview 'settings-ghost-mode-page' contains AI Ghost Mode settings.
+ * Ghost Mode silently learns from user browsing patterns and suggests automations.
+ */
+
+import '../settings_page/settings_section.js';
+import '../settings_shared.css.js';
+import '../controls/settings_toggle_button.js';
+import 'chrome://resources/cr_elements/cr_button/cr_button.js';
+import 'chrome://resources/cr_elements/cr_icon/cr_icon.js';
+import 'chrome://resources/cr_elements/cr_icon_button/cr_icon_button.js';
+import 'chrome://resources/cr_elements/icons.html.js';
+import 'chrome://resources/cr_elements/cr_shared_style.css.js';
+import 'chrome://resources/cr_elements/cr_dialog/cr_dialog.js';
+
+import {PrefsMixin} from '/shared/settings/prefs/prefs_mixin.js';
+import {PolymerElement} from 'chrome://resources/polymer/v3_0/polymer/polymer_bundled.min.js';
+
+import {getTemplate} from './ghost_mode_page.html.js';
+
+interface DetectedPattern {
+  id: string;
+  name: string;
+  occurrence_count: number;
+  confidence_score: number;
+  actions: Array<{type: string; url: string; selector?: string}>;
+  first_seen: string;
+  last_seen: string;
+}
+
+export interface SettingsGhostModePageElement {
+  $: {
+    addDomainDialog: HTMLElement;
+    domainInput: HTMLInputElement;
+    ghostModeToggle: HTMLElement;
+  };
+}
+
+const SettingsGhostModePageElementBase = PrefsMixin(PolymerElement);
+
+export class SettingsGhostModePageElement extends SettingsGhostModePageElementBase {
+  static get is() {
+    return 'settings-ghost-mode-page';
+  }
+
+  static get template() {
+    return getTemplate();
+  }
+
+  static get properties() {
+    return {
+      /**
+       * Preferences state.
+       */
+      prefs: {
+        type: Object,
+        notify: true,
+      },
+      
+      /**
+       * Number of actions recorded
+       */
+      actionsRecorded_: {
+        type: Number,
+        value: 0,
+      },
+      
+      /**
+       * Number of patterns detected
+       */
+      patternsDetected_: {
+        type: Number,
+        value: 0,
+      },
+      
+      /**
+       * Number of workflows generated
+       */
+      workflowsGenerated_: {
+        type: Number,
+        value: 0,
+      },
+      
+      /**
+       * List of detected patterns
+       */
+      patterns_: {
+        type: Array,
+        value: () => [],
+      },
+      
+      /**
+       * List of excluded domains
+       */
+      excludedDomains_: {
+        type: Array,
+        value: () => [],
+      },
+      
+      /**
+       * New domain for dialog
+       */
+      newDomain_: {
+        type: String,
+        value: '',
+      },
+    };
+  }
+
+  // Declare properties
+  declare prefs: any;
+  declare actionsRecorded_: number;
+  declare patternsDetected_: number;
+  declare workflowsGenerated_: number;
+  declare patterns_: DetectedPattern[];
+  declare excludedDomains_: string[];
+  declare newDomain_: string;
+
+  /**
+   * Initialize when attached to DOM
+   */
+  override connectedCallback() {
+    super.connectedCallback();
+    this.loadStats_();
+    this.loadPatterns_();
+    this.loadExcludedDomains_();
+  }
+  
+  /**
+   * Load statistics from Ghost Mode service
+   */
+  private loadStats_() {
+    // In production, this would call the GhostModeService
+    // For now, initialize with placeholder values
+    chrome.send('getGhostModeStats');
+  }
+  
+  /**
+   * Receive stats from backend
+   */
+  onGhostModeStatsReceived_(stats: {
+    actions: number;
+    patterns: number;
+    workflows: number;
+  }) {
+    this.actionsRecorded_ = stats.actions;
+    this.patternsDetected_ = stats.patterns;
+    this.workflowsGenerated_ = stats.workflows;
+  }
+  
+  /**
+   * Load detected patterns
+   */
+  private loadPatterns_() {
+    chrome.send('getGhostModePatterns');
+  }
+  
+  /**
+   * Receive patterns from backend
+   */
+  onGhostModePatternsReceived_(patterns: DetectedPattern[]) {
+    this.patterns_ = patterns;
+    this.patternsDetected_ = patterns.length;
+  }
+  
+  /**
+   * Load excluded domains from prefs
+   */
+  private loadExcludedDomains_() {
+    try {
+      const pref = this.getPref('ghost_mode.excluded_domains');
+      if (pref && pref.value) {
+        this.excludedDomains_ = JSON.parse(pref.value);
+      } else {
+        // Default excluded domains (banking, healthcare)
+        this.excludedDomains_ = [
+          'bankofamerica.com',
+          'chase.com',
+          'wellsfargo.com',
+          'citi.com',
+          'capitalone.com',
+          'mychart.com',
+          'patient.portal',
+        ];
+      }
+    } catch (e) {
+      console.warn('Failed to load excluded domains:', e);
+      this.excludedDomains_ = [];
+    }
+  }
+  
+  /**
+   * Save excluded domains to prefs
+   */
+  private saveExcludedDomains_() {
+    const domainsJson = JSON.stringify(this.excludedDomains_);
+    // @ts-ignore: setPrefValue exists at runtime from PrefsMixin
+    this.setPrefValue('ghost_mode.excluded_domains', domainsJson);
+  }
+  
+  /**
+   * Format confidence score as percentage
+   */
+  private formatConfidence_(score: number): string {
+    return `${Math.round(score * 100)}%`;
+  }
+  
+  /**
+   * Handle pattern click
+   */
+  private onPatternClick_(e: CustomEvent) {
+    const pattern = (e.target as any).closest('.pattern-item');
+    if (pattern) {
+      // Open pattern details dialog (future enhancement)
+      console.log('Pattern clicked:', e.model.item);
+    }
+  }
+  
+  /**
+   * Convert pattern to workflow
+   */
+  private convertToWorkflow_(e: Event) {
+    e.stopPropagation();
+    const pattern = (e.model as any).item as DetectedPattern;
+    chrome.send('convertPatternToWorkflow', [pattern.id]);
+  }
+  
+  /**
+   * Delete pattern
+   */
+  private deletePattern_(e: Event) {
+    e.stopPropagation();
+    const pattern = (e.model as any).item as DetectedPattern;
+    
+    if (confirm(`Delete pattern "${pattern.name}"? This cannot be undone.`)) {
+      chrome.send('deleteGhostModePattern', [pattern.id]);
+      this.patterns_ = this.patterns_.filter(p => p.id !== pattern.id);
+      this.patternsDetected_ = this.patterns_.length;
+    }
+  }
+  
+  /**
+   * Handle retention slider change
+   */
+  private onRetentionChange_(e: Event) {
+    const slider = e.target as HTMLInputElement;
+    const days = parseInt(slider.value, 10);
+    // @ts-ignore: setPrefValue exists at runtime from PrefsMixin
+    this.setPrefValue('ghost_mode.retention_days', days);
+  }
+  
+  /**
+   * Show add domain dialog
+   */
+  private showAddDomainDialog_() {
+    this.newDomain_ = '';
+    const dialog = this.$.addDomainDialog as any;
+    dialog.showModal();
+  }
+  
+  /**
+   * Cancel add domain dialog
+   */
+  private cancelAddDomain_() {
+    const dialog = this.$.addDomainDialog as any;
+    dialog.close();
+  }
+  
+  /**
+   * Confirm add domain
+   */
+  private confirmAddDomain_() {
+    const domain = this.newDomain_.trim().toLowerCase();
+    
+    if (domain && !this.excludedDomains_.includes(domain)) {
+      this.push('excludedDomains_', domain);
+      this.saveExcludedDomains_();
+    }
+    
+    this.cancelAddDomain_();
+  }
+  
+  /**
+   * Remove excluded domain
+   */
+  private removeDomain_(e: Event) {
+    const domain = (e.model as any).item as string;
+    this.excludedDomains_ = this.excludedDomains_.filter(d => d !== domain);
+    this.saveExcludedDomains_();
+  }
+  
+  /**
+   * Clear all Ghost Mode data
+   */
+  private clearAllData_() {
+    if (confirm('Are you sure you want to delete all Ghost Mode data? This includes all recorded actions and detected patterns. This action cannot be undone.')) {
+      chrome.send('clearGhostModeData');
+      this.actionsRecorded_ = 0;
+      this.patternsDetected_ = 0;
+      this.patterns_ = [];
+    }
+  }
+}
+
+declare global {
+  interface HTMLElementTagNameMap {
+    'settings-ghost-mode-page': SettingsGhostModePageElement;
+  }
+}
+
+customElements.define(
+  SettingsGhostModePageElement.is,
+  SettingsGhostModePageElement
+);
