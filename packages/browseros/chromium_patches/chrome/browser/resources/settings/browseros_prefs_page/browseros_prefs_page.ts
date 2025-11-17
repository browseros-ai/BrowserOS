diff --git a/chrome/browser/resources/settings/browseros_prefs_page/browseros_prefs_page.ts b/chrome/browser/resources/settings/browseros_prefs_page/browseros_prefs_page.ts
new file mode 100644
index 0000000000000..2616c77cc001b
--- /dev/null
+++ b/chrome/browser/resources/settings/browseros_prefs_page/browseros_prefs_page.ts
@@ -0,0 +1,250 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+/**
+ * @fileoverview 'settings-browseros-prefs-page' contains BlockBrowser-specific settings.
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
+import 'chrome://resources/cr_elements/cr_dialog/cr_dialog.js';
+
+import {PrefsMixin} from '/shared/settings/prefs/prefs_mixin.js';
+import {PolymerElement} from 'chrome://resources/polymer/v3_0/polymer/polymer_bundled.min.js';
+
+import {getTemplate} from './browseros_prefs_page.html.js';
+
+interface CustomProvider {
+  name: string;
+  url: string;
+}
+
+export interface SettingsBlockBrowserPrefsPageElement {
+  $: {
+    addProviderDialog: HTMLElement;
+  };
+}
+
+const SettingsBlockBrowserPrefsPageElementBase = PrefsMixin(PolymerElement);
+
+export class SettingsBlockBrowserPrefsPageElement extends SettingsBlockBrowserPrefsPageElementBase {
+  static get is() {
+    return 'settings-browseros-prefs-page';
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
+       * List of custom providers
+       */
+      customProviders: {
+        type: Array,
+        value: () => [],
+      },
+      
+      /**
+       * New provider name for dialog
+       */
+      newProviderName_: {
+        type: String,
+        value: '',
+      },
+      
+      /**
+       * New provider URL for dialog
+       */
+      newProviderUrl_: {
+        type: String,
+        value: '',
+      },
+    };
+  }
+
+  // Declare properties
+  declare prefs: any;
+  declare customProviders: CustomProvider[];
+  declare newProviderName_: string;
+  declare newProviderUrl_: string;
+
+  /**
+   * Initialize when attached to DOM
+   */
+  override connectedCallback() {
+    super.connectedCallback();
+    // Wait for prefs to be ready before loading
+    this.addEventListener('prefs-changed', () => {
+      if (this.prefs && !this.customProviders) {
+        this.loadCustomProviders_();
+      }
+    });
+    // Try loading immediately in case prefs are already available
+    if (this.prefs) {
+      this.loadCustomProviders_();
+    }
+  }
+  
+  /**
+   * Load custom providers from preference
+   */
+  private loadCustomProviders_() {
+    try {
+      const pref = this.getPref('browseros.custom_providers');
+      if (pref && pref.value) {
+        this.customProviders = JSON.parse(pref.value);
+      } else {
+        this.customProviders = [];
+      }
+    } catch (e) {
+      console.warn('Failed to load custom providers:', e);
+      this.customProviders = [];
+    }
+  }
+  
+  /**
+   * Save custom providers to preference
+   */
+  private saveCustomProviders_() {
+    const customProvidersJson = JSON.stringify(this.customProviders);
+    // @ts-ignore: setPrefValue exists at runtime from PrefsMixin
+    this.setPrefValue('browseros.custom_providers', customProvidersJson);
+  }
+  
+  /**
+   * Show add provider dialog
+   */
+  private showAddProviderDialog_() {
+    this.newProviderName_ = '';
+    this.newProviderUrl_ = '';
+    const dialog = this.$.addProviderDialog as any;
+    dialog.showModal();
+  }
+  
+  /**
+   * Cancel add provider dialog
+   */
+  private cancelAddProvider_() {
+    const dialog = this.$.addProviderDialog as any;
+    dialog.close();
+  }
+  
+  /**
+   * Add custom provider
+   */
+  private async addCustomProvider_() {
+    if (!this.newProviderName_ || !this.newProviderUrl_) {
+      return;
+    }
+    
+    // Validate URL
+    try {
+      const url = new URL(this.newProviderUrl_);
+      if (!url.protocol.startsWith('http')) {
+        alert('Please enter a valid HTTP or HTTPS URL');
+        return;
+      }
+    } catch (e) {
+      alert('Please enter a valid URL');
+      return;
+    }
+    
+    // Add the new provider
+    this.push('customProviders', {
+      name: this.newProviderName_,
+      url: this.newProviderUrl_
+    });
+    
+    // Save to preferences
+    this.saveCustomProviders_();
+    
+    // Clear input fields
+    this.newProviderName_ = '';
+    this.newProviderUrl_ = '';
+    
+    // Close dialog with small delay
+    await new Promise(resolve => setTimeout(resolve, 50));
+    this.cancelAddProvider_();
+    
+    // Show status message after dialog closes
+    await new Promise(resolve => setTimeout(resolve, 100));
+    this.showStatusMessage_();
+  }
+  
+  /**
+   * Delete custom provider
+   */
+  private deleteCustomProvider_(e: Event) {
+    const button = e.currentTarget as HTMLElement;
+    const index = parseInt(button.dataset['index'] || '0');
+    
+    // Remove the provider
+    this.splice('customProviders', index, 1);
+    
+    // Save to preferences
+    this.saveCustomProviders_();
+    
+    // Show status message
+    this.showStatusMessage_();
+  }
+  
+  /**
+   * Handle toolbar label toggle change
+   */
+  override ready() {
+    super.ready();
+    
+    // Listen for toggle changes
+    const toggleButton = this.shadowRoot!.querySelector('#showToolbarLabels');
+    if (toggleButton) {
+      toggleButton.addEventListener('change', () => {
+        // The toggle button will automatically update the pref through PrefsMixin
+        this.showStatusMessage_();
+      });
+    }
+  }
+  
+  /**
+   * Show status message briefly
+   */
+  private showStatusMessage_() {
+    const statusMessage = this.shadowRoot!.querySelector('#statusMessage') as HTMLElement;
+    if (statusMessage) {
+      // Remove class first to reset animation if called multiple times
+      statusMessage.classList.remove('show');
+      // Force reflow
+      void statusMessage.offsetWidth;
+      // Add class to show
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
+    'settings-browseros-prefs-page': SettingsBlockBrowserPrefsPageElement;
+  }
+}
+
+customElements.define(
+    SettingsBlockBrowserPrefsPageElement.is, SettingsBlockBrowserPrefsPageElement);
\ No newline at end of file
