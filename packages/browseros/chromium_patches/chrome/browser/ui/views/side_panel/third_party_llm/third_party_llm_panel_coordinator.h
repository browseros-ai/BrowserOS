diff --git a/chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h b/chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h
new file mode 100644
index 0000000000000..1b30c5d723c09
--- /dev/null
+++ b/chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h
@@ -0,0 +1,236 @@
+// Copyright 2026 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#ifndef CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_THIRD_PARTY_LLM_THIRD_PARTY_LLM_PANEL_COORDINATOR_H_
+#define CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_THIRD_PARTY_LLM_THIRD_PARTY_LLM_PANEL_COORDINATOR_H_
+
+#include <map>
+#include <string>
+
+#include "base/memory/raw_ptr.h"
+#include "base/memory/raw_ref.h"
+#include "base/memory/weak_ptr.h"
+#include "base/scoped_multi_source_observation.h"
+#include "base/scoped_observation.h"
+#include "base/timer/timer.h"
+#include "chrome/browser/ui/browser_list_observer.h"
+#include "chrome/browser/profiles/profile_observer.h"
+#include "components/prefs/pref_change_registrar.h"
+#include "content/public/browser/media_stream_request.h"
+#include "content/public/browser/web_contents_delegate.h"
+#include "content/public/browser/web_contents_observer.h"
+#include "third_party/blink/public/mojom/choosers/file_chooser.mojom.h"
+#include "third_party/blink/public/mojom/mediastream/media_stream.mojom.h"
+#include "third_party/blink/public/mojom/window_features/window_features.mojom.h"
+#include "ui/accessibility/ax_node_id_forward.h"
+#include "ui/base/window_open_disposition.h"
+#include "ui/views/controls/webview/unhandled_keyboard_event_handler.h"
+#include "url/gurl.h"
+#include "ui/views/view_observer.h"
+#include "ui/menus/simple_menu_model.h"
+
+namespace gfx {
+class Image;
+}  // namespace gfx
+
+class BrowserList;
+class Profile;
+class SidePanelEntryScope;
+class SidePanelRegistry;
+class TabStripModel;
+
+namespace input {
+struct NativeWebKeyboardEvent;
+}  // namespace input
+
+namespace user_prefs {
+class PrefRegistrySyncable;
+}  // namespace user_prefs
+
+namespace content {
+class FileSelectListener;
+class RenderFrameHost;
+class WebContents;
+}  // namespace content
+
+namespace ui {
+struct AXNodeData;
+struct AXTreeUpdate;
+}  // namespace ui
+
+namespace views {
+class Combobox;
+class ImageButton;
+class Label;
+class MenuRunner;
+class View;
+class WebView;
+}  // namespace views
+
+// Information about an LLM provider
+struct LlmProviderInfo {
+  std::u16string name;  // Display name (e.g., "ChatGPT")
+  GURL url;            // Provider URL
+};
+
+// ThirdPartyLlmPanelCoordinator handles the creation and registration of the
+// third-party LLM SidePanelEntry.
+class ThirdPartyLlmPanelCoordinator
+    : public BrowserListObserver,
+      public ProfileObserver,
+      public content::WebContentsDelegate,
+      public content::WebContentsObserver,
+      public views::ViewObserver,
+      public ui::SimpleMenuModel::Delegate {
+ public:
+  ThirdPartyLlmPanelCoordinator(Profile* profile, TabStripModel* tab_strip_model);
+  ThirdPartyLlmPanelCoordinator(const ThirdPartyLlmPanelCoordinator&) = delete;
+  ThirdPartyLlmPanelCoordinator& operator=(const ThirdPartyLlmPanelCoordinator&) = delete;
+  ~ThirdPartyLlmPanelCoordinator() override;
+
+  void CreateAndRegisterEntry(SidePanelRegistry* global_registry);
+
+  // Registers user preferences
+  static void RegisterProfilePrefs(user_prefs::PrefRegistrySyncable* registry);
+
+  // Cycles to the next LLM provider
+  void CycleProvider();
+
+  Profile* GetProfile() const { return &profile_.get(); }
+  TabStripModel* GetTabStripModel() const { return &tab_strip_model_.get(); }
+  
+  // content::WebContentsDelegate:
+  bool HandleKeyboardEvent(content::WebContents* source,
+                          const input::NativeWebKeyboardEvent& event) override;
+  content::WebContents* AddNewContents(
+      content::WebContents* source,
+      std::unique_ptr<content::WebContents> new_contents,
+      const GURL& target_url,
+      WindowOpenDisposition disposition,
+      const blink::mojom::WindowFeatures& window_features,
+      bool user_gesture,
+      bool* was_blocked) override;
+  void RunFileChooser(content::RenderFrameHost* render_frame_host,
+                      scoped_refptr<content::FileSelectListener> listener,
+                      const blink::mojom::FileChooserParams& params) override;
+  void RequestMediaAccessPermission(
+      content::WebContents* web_contents,
+      const content::MediaStreamRequest& request,
+      content::MediaResponseCallback callback) override;
+  bool CheckMediaAccessPermission(
+      content::RenderFrameHost* render_frame_host,
+      const url::Origin& security_origin,
+      blink::mojom::MediaStreamType type) override;
+  
+  // content::WebContentsObserver:
+  void DidFinishLoad(content::RenderFrameHost* render_frame_host,
+                     const GURL& validated_url) override;
+
+  // views::ViewObserver:
+  void OnViewIsDeleting(views::View* observed_view) override;
+
+  // BrowserListObserver:
+  void OnBrowserRemoved(Browser* browser) override;
+
+  // ProfileObserver:
+  void OnProfileWillBeDestroyed(Profile* profile) override;
+
+  // ui::SimpleMenuModel::Delegate:
+  void ExecuteCommand(int command_id, int event_flags) override;
+
+ private:
+
+  // Menu command IDs
+  enum MenuCommands {
+    IDC_COPY_CONTENT = 1,
+    IDC_SCREENSHOT,
+    IDC_REFRESH,
+    IDC_OPEN_IN_NEW_TAB,
+  };
+
+  std::unique_ptr<views::View> CreateThirdPartyLlmWebView(
+      SidePanelEntryScope& scope);
+
+  void OnProviderChanged();
+  void OnRefreshContent();
+  void OnOpenInNewTab();
+  void OnCopyContent();
+  void OnScreenshotContent();
+  void OnAccessibilityTreeReceived(ui::AXTreeUpdate& update);
+  void OnScreenshotCaptured(const gfx::Image& image);
+  void ExtractTextFromNodeData(
+      const ui::AXNodeData* node,
+      const std::map<ui::AXNodeID, const ui::AXNodeData*>& node_map,
+      std::u16string* output);
+  void HideFeedbackLabel();
+  void ShowOptionsMenu();
+
+  // Provider management
+  std::vector<LlmProviderInfo> GetDefaultProviders() const;
+  void LoadProvidersFromPrefs();
+  void SaveProvidersToPrefs();
+
+  // Executes the actual provider switch after all sanity checks. Should only
+  // be called on the UI thread.  Uses |provider_change_in_progress_| to avoid
+  // reentrancy.
+  void DoProviderChange(size_t new_provider_index);
+
+  // Clean up WebContents early to avoid shutdown crashes.
+  void CleanupWebContents();
+
+  const raw_ref<Profile> profile_;
+  const raw_ref<TabStripModel> tab_strip_model_;
+
+  // Provider list and current selection
+  std::vector<LlmProviderInfo> providers_;
+  size_t current_provider_index_ = 0;
+
+  // UI elements
+  raw_ptr<views::WebView> web_view_ = nullptr;
+  raw_ptr<views::Combobox> provider_selector_ = nullptr;
+  raw_ptr<views::Label> copy_feedback_label_ = nullptr;
+  raw_ptr<views::ImageButton> menu_button_ = nullptr;
+
+  // We need to own the WebContents because WebView doesn't take ownership
+  // when we call SetWebContents with externally created WebContents
+  std::unique_ptr<content::WebContents> owned_web_contents_;
+
+  // Store the last URL for each provider to restore state
+  std::map<size_t, GURL> last_urls_;
+  
+  // Timer for auto-hiding feedback messages
+  std::unique_ptr<base::OneShotTimer> feedback_timer_;
+  
+  // Temporary storage for page info during copy
+  std::u16string page_title_;
+  GURL page_url_;
+  
+  // Handler for unhandled keyboard events
+  views::UnhandledKeyboardEventHandler unhandled_keyboard_event_handler_;
+  
+  // Reentrancy guard to prevent nested/overlapping provider changes that can
+  // leave the combobox selection and WebView out of sync.
+  bool provider_change_in_progress_ = false;
+  
+  // Observe lifetime of UI views we hold raw pointers to so that we can
+  // null-check safely after they are destroyed (e.g. when the side panel is
+  // closed). This prevents dangling pointer dereference from delayed tasks.
+  base::ScopedMultiSourceObservation<views::View, views::ViewObserver>
+      view_observation_{this};
+
+  // Observer registrations for early cleanup notifications
+  base::ScopedObservation<BrowserList, BrowserListObserver>
+      browser_list_observation_{this};
+  base::ScopedObservation<Profile, ProfileObserver>
+      profile_observation_{this};
+
+  // Menu model and runner for options menu
+  std::unique_ptr<ui::SimpleMenuModel> menu_model_;
+  std::unique_ptr<views::MenuRunner> menu_runner_;
+  
+  // Weak pointer factory for callbacks
+  base::WeakPtrFactory<ThirdPartyLlmPanelCoordinator> weak_factory_{this};
+};
+
+#endif  // CHROME_BROWSER_UI_VIEWS_SIDE_PANEL_THIRD_PARTY_LLM_THIRD_PARTY_LLM_PANEL_COORDINATOR_H_
