diff --git a/chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_coordinator.cc b/chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_coordinator.cc
new file mode 100644
index 0000000000000..9e1c6dc9fe8a1
--- /dev/null
+++ b/chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_coordinator.cc
@@ -0,0 +1,568 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_coordinator.h"
+
+#include "base/check.h"
+#include "base/functional/bind.h"
+#include "base/logging.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/stringprintf.h"
+#include "base/strings/utf_string_conversions.h"
+#include "base/task/sequenced_task_runner.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/ui/browser.h"
+#include "chrome/browser/ui/browser_commands.h"
+#include "chrome/browser/ui/browser_list.h"
+#include "chrome/browser/ui/browser_tabstrip.h"
+#include "chrome/browser/ui/browser_window.h"
+#include "chrome/browser/ui/tabs/tab_strip_model.h"
+#include "chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_view.h"
+#include "chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_window.h"
+#include "components/input/native_web_keyboard_event.h"
+#include "components/pref_registry/pref_registry_syncable.h"
+#include "components/prefs/pref_service.h"
+#include "components/prefs/scoped_user_pref_update.h"
+#include "content/public/browser/render_frame_host.h"
+#include "content/public/browser/web_contents.h"
+#include "ui/base/clipboard/clipboard.h"
+#include "ui/base/clipboard/scoped_clipboard_writer.h"
+#include "ui/accessibility/ax_tree_update.h"
+#include "ui/events/keycodes/keyboard_codes.h"
+#include "third_party/blink/public/common/input/web_input_event.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+
+namespace {
+
+// Preference names
+const char kClashOfGptsPaneProvidersPref[] = "browseros.clash_of_gpts.pane_providers";  // Per-pane selections
+const char kClashOfGptsLastUrlsPref[] = "browseros.clash_of_gpts.last_urls";
+const char kClashOfGptsPaneCountPref[] = "browseros.clash_of_gpts.pane_count";
+
+// Shared provider list preference (from third_party_llm)
+const char kThirdPartyLlmProvidersPref[] = "browseros.third_party_llm.providers";
+
+}  // namespace
+
+ClashOfGptsCoordinator::ClashOfGptsCoordinator(Browser* browser)
+    : browser_(browser) {
+  CHECK(browser_);
+  // Register for early cleanup notifications
+  browser_list_observation_.Observe(BrowserList::GetInstance());
+  profile_observation_.Observe(browser_->profile());
+
+  // Load shared provider list first
+  LoadProvidersFromPrefs();
+
+  // Initialize with default provider indices for max panes
+  pane_provider_indices_[0] = 0;
+  pane_provider_indices_[1] = 1;
+  pane_provider_indices_[2] = 2;
+
+  LoadState();
+}
+
+ClashOfGptsCoordinator::~ClashOfGptsCoordinator() {
+  // Destructor should be minimal - cleanup already done in observer methods
+  // The ScopedObservation objects will automatically unregister
+  SaveState();
+}
+
+void ClashOfGptsCoordinator::Show() {
+  CreateWindowIfNeeded();
+  if (widget_) {
+    widget_->Show();
+    widget_->Activate();
+    browseros_metrics::BrowserOSMetrics::Log("llmhub.shown");
+  }
+}
+
+void ClashOfGptsCoordinator::Close() {
+  if (widget_) {
+    // Following Chromium style guide: destroy widget by resetting unique_ptr
+    widget_.reset();
+  }
+  window_.reset();
+  view_ = nullptr;
+}
+
+bool ClashOfGptsCoordinator::IsShowing() const {
+  return widget_ && widget_->IsVisible();
+}
+
+void ClashOfGptsCoordinator::CycleProviderInPane(int pane_index) {
+  if (pane_index < 0 || pane_index >= current_pane_count_) {
+    return;
+  }
+
+  if (providers_.empty()) {
+    return;
+  }
+
+  size_t current = pane_provider_indices_[pane_index];
+  size_t next = (current + 1) % providers_.size();
+  SetProviderForPane(pane_index, next);
+}
+
+void ClashOfGptsCoordinator::CopyContentToAll() {
+  // Get the active tab's web contents
+  TabStripModel* tab_strip_model = GetBrowser().tab_strip_model();
+  if (!tab_strip_model) {
+    return;
+  }
+
+  content::WebContents* active_contents = tab_strip_model->GetActiveWebContents();
+  if (!active_contents) {
+    return;
+  }
+
+  // Get the page title and URL
+  std::u16string page_title = active_contents->GetTitle();
+  GURL page_url = active_contents->GetVisibleURL();
+
+  // Request accessibility tree snapshot (similar to the side panel implementation)
+  active_contents->RequestAXTreeSnapshot(
+      base::BindOnce([](std::u16string title, GURL url, ui::AXTreeUpdate& update) {
+        // Extract text from accessibility tree
+        std::u16string extracted_text;
+        // TODO: Implement text extraction similar to third_party_llm_panel_coordinator.cc
+        
+        // Format the output for comparison across LLMs
+        std::u16string formatted_output = u"----------- WEB PAGE CONTENT -----------\n\n";
+        formatted_output += u"TITLE: " + title + u"\n\n";
+        formatted_output += u"URL: " + base::UTF8ToUTF16(url.spec()) + u"\n\n";
+        formatted_output += u"CONTENT:\n\n" + extracted_text;
+        formatted_output += u"\n\n----------- USER PROMPT -----------\n\n";
+
+        // Copy to clipboard
+        ui::ScopedClipboardWriter clipboard_writer(ui::ClipboardBuffer::kCopyPaste);
+        clipboard_writer.WriteText(formatted_output);
+      }, page_title, page_url),
+      ui::AXMode::kWebContents,
+      0,  // max_nodes (0 = no limit)
+      base::Seconds(5),  // timeout
+      content::WebContents::AXTreeSnapshotPolicy::kSameOriginDirectDescendants);
+
+  // Show feedback in the UI
+  if (view_) {
+    view_->ShowCopyFeedback();
+  }
+}
+
+std::vector<LlmProviderInfo> ClashOfGptsCoordinator::GetDefaultProviders() const {
+  std::vector<LlmProviderInfo> defaults;
+  defaults.push_back({u"ChatGPT", GURL("https://chatgpt.com")});
+  defaults.push_back({u"Claude", GURL("https://claude.ai")});
+  defaults.push_back({u"Grok", GURL("https://grok.com")});
+  defaults.push_back({u"Gemini", GURL("https://gemini.google.com")});
+  defaults.push_back({u"Perplexity", GURL("https://www.perplexity.ai")});
+  return defaults;
+}
+
+void ClashOfGptsCoordinator::LoadProvidersFromPrefs() {
+  PrefService* prefs = GetBrowser().profile()->GetPrefs();
+  if (!prefs) {
+    LOG(ERROR) << "[browseros] Failed to get PrefService";
+    providers_ = GetDefaultProviders();
+    return;
+  }
+
+  const base::Value::List& providers_list = prefs->GetList(kThirdPartyLlmProvidersPref);
+
+  providers_.clear();
+
+  if (!providers_list.empty()) {
+    for (const base::Value& item : providers_list) {
+      if (!item.is_dict()) {
+        LOG(WARNING) << "[browseros] Invalid provider entry (not a dict), skipping";
+        continue;
+      }
+
+      const std::string* name = item.GetDict().FindString("name");
+      const std::string* url = item.GetDict().FindString("url");
+
+      if (!name || name->empty()) {
+        LOG(WARNING) << "[browseros] Provider missing name, skipping";
+        continue;
+      }
+
+      if (!url || url->empty()) {
+        LOG(WARNING) << "[browseros] Provider missing URL, skipping";
+        continue;
+      }
+
+      GURL provider_url(*url);
+      if (!provider_url.is_valid()) {
+        LOG(WARNING) << "[browseros] Invalid provider URL: " << *url;
+        continue;
+      }
+
+      providers_.push_back({base::UTF8ToUTF16(*name), provider_url});
+    }
+  }
+
+  // If no valid providers loaded, use defaults
+  if (providers_.empty()) {
+    LOG(INFO) << "[browseros] No providers in prefs, using defaults";
+    providers_ = GetDefaultProviders();
+  }
+}
+
+size_t ClashOfGptsCoordinator::GetProviderIndexForPane(int pane_index) const {
+  if (pane_index < 0 || pane_index >= current_pane_count_) {
+    return 0;  // Default to first provider
+  }
+  return pane_provider_indices_[pane_index];
+}
+
+void ClashOfGptsCoordinator::SetProviderForPane(int pane_index, size_t provider_index) {
+  if (pane_index < 0 || pane_index >= current_pane_count_) {
+    return;
+  }
+
+  if (provider_index >= providers_.size()) {
+    LOG(ERROR) << "[browseros] Invalid provider index: " << provider_index;
+    return;
+  }
+
+  // Save the current URL for this pane/provider combo
+  if (view_) {
+    if (content::WebContents* web_contents = view_->GetWebContentsForPane(pane_index)) {
+      GURL current_url = web_contents->GetURL();
+      if (current_url.is_valid()) {
+        last_urls_[{pane_index, pane_provider_indices_[pane_index]}] = current_url;
+      }
+    }
+  }
+
+  pane_provider_indices_[pane_index] = provider_index;
+  SaveState();
+
+  // Navigate to the new provider URL
+  if (view_) {
+    GURL provider_url;
+    auto it = last_urls_.find({pane_index, provider_index});
+    if (it != last_urls_.end() && it->second.is_valid()) {
+      provider_url = it->second;
+    } else {
+      provider_url = providers_[provider_index].url;
+    }
+    view_->NavigatePaneToUrl(pane_index, provider_url);
+  }
+}
+
+void ClashOfGptsCoordinator::SetPaneCount(int count) {
+  if (count < kMinPanes || count > kMaxPanes || count == current_pane_count_) {
+    return;
+  }
+
+  current_pane_count_ = count;
+  SaveState();
+  
+  browseros_metrics::BrowserOSMetrics::Log("llmhub.panecount.changed", 
+    {{"count", base::Value(count)}});
+
+  // Update the view if it exists
+  if (view_) {
+    view_->UpdatePaneCount(count);
+  }
+
+  // Resize window based on new pane count
+  if (widget_ && widget_->IsVisible()) {
+    // int window_width = current_pane_count_ == 2 ? 1000 : 1400;
+    int window_width = 1400;
+    gfx::Size new_size(window_width, widget_->GetWindowBoundsInScreen().height());
+    widget_->CenterWindow(new_size);
+  }
+}
+
+void ClashOfGptsCoordinator::CreateAndRegisterEntry(SidePanelRegistry* registry) {
+  // For now, we don't register a side panel entry since Clash of GPTs
+  // opens in its own window. This method is here for compatibility
+  // with the side panel infrastructure.
+}
+
+
+bool ClashOfGptsCoordinator::HandleKeyboardEvent(
+    content::WebContents* source,
+    const input::NativeWebKeyboardEvent& event) {
+  // Use the unhandled keyboard event handler to process the event
+  // This ensures standard browser shortcuts like Cmd+C and Cmd+V work properly
+  if (view_ && view_->GetWidget()) {
+    views::FocusManager* focus_manager = view_->GetWidget()->GetFocusManager();
+    if (focus_manager) {
+      return unhandled_keyboard_event_handler_.HandleKeyboardEvent(event, focus_manager);
+    }
+  }
+
+  return false;
+}
+
+content::WebContents* ClashOfGptsCoordinator::AddNewContents(
+    content::WebContents* source,
+    std::unique_ptr<content::WebContents> new_contents,
+    const GURL& target_url,
+    WindowOpenDisposition disposition,
+    const blink::mojom::WindowFeatures& window_features,
+    bool user_gesture,
+    bool* was_blocked) {
+  // Handle popup windows from the webviews
+  if (!user_gesture) {
+    if (was_blocked) {
+      *was_blocked = true;
+    }
+    return nullptr;
+  }
+
+  // For popup windows and new tabs, open them in the main browser
+  if (disposition == WindowOpenDisposition::NEW_POPUP ||
+      disposition == WindowOpenDisposition::NEW_FOREGROUND_TAB ||
+      disposition == WindowOpenDisposition::NEW_BACKGROUND_TAB ||
+      disposition == WindowOpenDisposition::NEW_WINDOW) {
+    chrome::AddWebContents(&GetBrowser(), source, std::move(new_contents),
+                          target_url, disposition, window_features);
+  }
+
+  return nullptr;
+}
+
+void ClashOfGptsCoordinator::OnViewIsDeleting(views::View* observed_view) {
+  if (observed_view == view_) {
+    view_ = nullptr;
+  }
+  view_observation_.RemoveObservation(observed_view);
+}
+
+void ClashOfGptsCoordinator::CreateWindowIfNeeded() {
+  LOG(INFO) << "CreateWindowIfNeeded called, window_ = " << window_.get();
+
+  if (!window_) {
+    LOG(INFO) << "Creating new window and widget";
+
+    // Reload providers from preferences (may have been modified externally)
+    size_t previous_size = providers_.size();
+    LoadProvidersFromPrefs();
+
+    // If provider list size changed
+    if (providers_.size() != previous_size) {
+      LOG(INFO) << "[browseros] Provider list size changed from " << previous_size
+                << " to " << providers_.size() << ", validating pane indices";
+      for (int i = 0; i < kMaxPanes; ++i) {
+        if (pane_provider_indices_[i] >= providers_.size()) {
+          pane_provider_indices_[i] = 0;  // Reset invalid indices
+        }
+      }
+    }
+
+    // Following Chromium style guide: CLIENT_OWNS_WIDGET pattern
+    // Client (coordinator) owns both widget and delegate separately
+    window_ = std::make_unique<ClashOfGptsWindow>(&GetBrowser(), this);
+    
+    // Create and store the widget
+    widget_ = std::make_unique<views::Widget>();
+    views::Widget::InitParams params(views::Widget::InitParams::CLIENT_OWNS_WIDGET,
+                                     views::Widget::InitParams::TYPE_WINDOW);
+    params.delegate = window_.get();
+    params.name = "ClashOfGptsWindow";
+    // Calculate window size based on pane count
+    // For 2 panes: ~1000px width, for 3 panes: ~1400px width
+    // int window_width = current_pane_count_ == 2 ? 1000 : 1400;
+    int window_width = 1400;
+    gfx::Size window_size(window_width, 800);
+    
+    params.bounds = gfx::Rect(window_size);
+    widget_->Init(std::move(params));
+    
+    // Let the window know about its widget
+    window_->SetWidget(widget_.get());
+    
+    // Center the window
+    if (GetBrowser().window()) {
+      widget_->CenterWindow(window_size);
+    }
+    
+    view_ = window_->GetView();
+    if (view_) {
+      view_observation_.AddObservation(view_);
+    }
+    
+    LOG(INFO) << "Window and widget creation complete";
+  } else {
+    LOG(INFO) << "Window already exists, not creating new one";
+  }
+}
+
+void ClashOfGptsCoordinator::SaveState() {
+  PrefService* prefs = GetBrowser().profile()->GetPrefs();
+  if (!prefs) {
+    return;
+  }
+
+  // Save pane count
+  prefs->SetInteger(kClashOfGptsPaneCountPref, current_pane_count_);
+
+  // Save provider selections (as indices)
+  ScopedListPrefUpdate providers_update(prefs, kClashOfGptsPaneProvidersPref);
+  providers_update->clear();
+  for (int i = 0; i < current_pane_count_; ++i) {
+    providers_update->Append(static_cast<int>(pane_provider_indices_[i]));
+  }
+
+  // Save last URLs
+  ScopedDictPrefUpdate urls_update(prefs, kClashOfGptsLastUrlsPref);
+  urls_update->clear();
+  for (const auto& [key, url] : last_urls_) {
+    std::string dict_key = base::StringPrintf("%d_%zu", key.first, key.second);
+    urls_update->Set(dict_key, url.spec());
+  }
+}
+
+void ClashOfGptsCoordinator::LoadState() {
+  PrefService* prefs = GetBrowser().profile()->GetPrefs();
+  if (!prefs) {
+    // Use defaults - already initialized in constructor
+    return;
+  }
+
+  // Load pane count
+  current_pane_count_ = prefs->GetInteger(kClashOfGptsPaneCountPref);
+  if (current_pane_count_ < kMinPanes || current_pane_count_ > kMaxPanes) {
+    current_pane_count_ = kDefaultPaneCount;
+  }
+
+  // Load provider selections (indices)
+  const base::Value::List& providers_list = prefs->GetList(kClashOfGptsPaneProvidersPref);
+  if (providers_list.size() > 0) {
+    for (size_t i = 0; i < providers_list.size() && i < kMaxPanes; ++i) {
+      if (providers_list[i].is_int()) {
+        int provider_index = providers_list[i].GetInt();
+        if (provider_index >= 0 && static_cast<size_t>(provider_index) < providers_.size()) {
+          pane_provider_indices_[i] = static_cast<size_t>(provider_index);
+        }
+      }
+    }
+  }
+
+  // Load last URLs
+  const base::Value::Dict& urls_dict = prefs->GetDict(kClashOfGptsLastUrlsPref);
+  for (const auto [key, value] : urls_dict) {
+    if (const std::string* url_str = value.GetIfString()) {
+      // Parse key format "pane_provider" safely without sscanf
+      size_t underscore_pos = key.find('_');
+      if (underscore_pos != std::string::npos && underscore_pos > 0 &&
+          underscore_pos < key.length() - 1) {
+        int pane_index, provider_index;
+        if (base::StringToInt(key.substr(0, underscore_pos), &pane_index) &&
+            base::StringToInt(key.substr(underscore_pos + 1), &provider_index)) {
+          if (pane_index >= 0 && pane_index < kMaxPanes &&
+              provider_index >= 0 && static_cast<size_t>(provider_index) < providers_.size()) {
+            GURL url(*url_str);
+            if (url.is_valid()) {
+              last_urls_[{pane_index, static_cast<size_t>(provider_index)}] = url;
+            }
+          }
+        }
+      }
+    }
+  }
+}
+
+// static
+void ClashOfGptsCoordinator::RegisterProfilePrefs(
+    user_prefs::PrefRegistrySyncable* registry) {
+  registry->RegisterListPref(kClashOfGptsPaneProvidersPref);
+  registry->RegisterDictionaryPref(kClashOfGptsLastUrlsPref);
+  registry->RegisterIntegerPref(kClashOfGptsPaneCountPref, kDefaultPaneCount);
+}
+
+
+// PaneWebContentsObserver implementation
+ClashOfGptsCoordinator::PaneWebContentsObserver::PaneWebContentsObserver(
+    ClashOfGptsCoordinator* coordinator, content::WebContents* web_contents)
+    : content::WebContentsObserver(web_contents),
+      coordinator_(coordinator) {}
+
+ClashOfGptsCoordinator::PaneWebContentsObserver::~PaneWebContentsObserver() = default;
+
+void ClashOfGptsCoordinator::PaneWebContentsObserver::DidFinishLoad(
+    content::RenderFrameHost* render_frame_host,
+    const GURL& validated_url) {
+  // Nothing to do on page load
+}
+
+content::WebContents* ClashOfGptsCoordinator::GetOrCreateWebContentsForPane(int pane_index) {
+  if (pane_index < 0 || pane_index >= kMaxPanes) {
+    return nullptr;
+  }
+
+  if (!owned_web_contents_[pane_index]) {
+    content::WebContents::CreateParams params(GetBrowser().profile());
+    owned_web_contents_[pane_index] = content::WebContents::Create(params);
+
+    // Set this as the delegate to handle keyboard events
+    owned_web_contents_[pane_index]->SetDelegate(this);
+
+    // Create observer for this pane
+    pane_observers_[pane_index] = std::make_unique<PaneWebContentsObserver>(
+        this, owned_web_contents_[pane_index].get());
+  }
+
+  return owned_web_contents_[pane_index].get();
+}
+
+void ClashOfGptsCoordinator::CleanupWebContents() {
+  // Save any URLs before cleanup
+  if (view_) {
+    for (int i = 0; i < current_pane_count_; ++i) {
+      if (content::WebContents* web_contents = view_->GetWebContentsForPane(i)) {
+        GURL current_url = web_contents->GetURL();
+        if (current_url.is_valid()) {
+          last_urls_[{i, pane_provider_indices_[i]}] = current_url;
+        }
+      }
+    }
+  }
+
+  // Clear all WebContents first
+  for (int i = 0; i < kMaxPanes; ++i) {
+    // Clear the observer first
+    pane_observers_[i].reset();
+
+    // Then destroy the WebContents
+    owned_web_contents_[i].reset();
+  }
+
+  // Remove view observation before widget cleanup
+  if (view_) {
+    view_observation_.RemoveObservation(view_);
+    view_ = nullptr;
+  }
+
+  // Close the window if it exists
+  if (widget_ && !widget_->IsClosed()) {
+    widget_->CloseNow();
+  }
+  widget_.reset();
+  window_.reset();
+}
+
+void ClashOfGptsCoordinator::OnBrowserRemoved(Browser* browser) {
+  if (browser == browser_) {
+    // Browser is being removed - clean up WebContents early
+    CleanupWebContents();
+  }
+}
+
+void ClashOfGptsCoordinator::OnProfileWillBeDestroyed(Profile* profile) {
+  if (profile == browser_->profile()) {
+    // Profile is being destroyed - clean up WebContents if not already done
+    CleanupWebContents();
+  }
+}
+
+Browser& ClashOfGptsCoordinator::GetBrowser() const {
+  CHECK(browser_);
+  return *browser_;
+}
