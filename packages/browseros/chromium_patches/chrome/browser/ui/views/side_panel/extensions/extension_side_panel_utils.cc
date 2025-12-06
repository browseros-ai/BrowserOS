diff --git a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_utils.cc b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_utils.cc
index ff61e95a7bba9..312c019f68442 100644
--- a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_utils.cc
+++ b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_utils.cc
@@ -4,6 +4,7 @@
 
 #include "chrome/browser/ui/extensions/extension_side_panel_utils.h"
 
+#include "base/logging.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_features.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
@@ -216,4 +217,127 @@ void CloseContextualExtensionSidePanel(BrowserWindowInterface* browser_window,
   }
 }
 
+bool IsContextualExtensionSidePanelOpen(BrowserWindowInterface* browser_window,
+                                        content::WebContents* web_contents,
+                                        const ExtensionId& extension_id) {
+  LOG(INFO) << "browseros: IsContextualExtensionSidePanelOpen called for extension="
+            << extension_id;
+
+  if (!browser_window || !web_contents) {
+    LOG(WARNING) << "browseros: browser_window or web_contents is null";
+    return false;
+  }
+
+  const SidePanelEntry::Key extension_key(SidePanelEntry::Id::kExtension,
+                                          extension_id);
+
+  content::WebContents* active_web_contents =
+      browser_window->GetActiveTabInterface()->GetContents();
+
+  bool is_active_tab = (web_contents == active_web_contents);
+  LOG(INFO) << "browseros: is_active_tab=" << is_active_tab;
+
+  // If this is the active tab, check if the side panel is currently showing
+  // this extension's entry.
+  if (is_active_tab) {
+    SidePanelUI* side_panel_ui = browser_window->GetFeatures().side_panel_ui();
+    bool is_showing = side_panel_ui->IsSidePanelShowing();
+    LOG(INFO) << "browseros: side_panel is_showing=" << is_showing;
+    if (!is_showing) {
+      return false;
+    }
+    // Check if it's this extension's contextual panel that's showing.
+    tabs::TabInterface* tab = tabs::TabInterface::GetFromContents(web_contents);
+    SidePanelRegistry* contextual_registry =
+        tab->GetTabFeatures()->side_panel_registry();
+    bool is_active = IsKeyActiveInRegistry(contextual_registry, extension_key);
+    LOG(INFO) << "browseros: contextual panel is_active=" << is_active;
+    return is_active;
+  }
+
+  // For inactive tabs, check if the contextual panel is set as active
+  // (it will show when the tab becomes active).
+  tabs::TabInterface* tab = tabs::TabInterface::GetFromContents(web_contents);
+  SidePanelRegistry* contextual_registry =
+      tab->GetTabFeatures()->side_panel_registry();
+  bool is_active = IsKeyActiveInRegistry(contextual_registry, extension_key);
+  LOG(INFO) << "browseros: inactive tab contextual panel is_active=" << is_active;
+  return is_active;
+}
+
+bool ToggleContextualExtensionSidePanel(BrowserWindowInterface& browser_window,
+                                        content::WebContents& web_contents,
+                                        const ExtensionId& extension_id,
+                                        std::optional<bool> desired_state) {
+  LOG(INFO) << "browseros: ToggleContextualExtensionSidePanel called for extension="
+            << extension_id << ", desired_state="
+            << (desired_state.has_value() ? (desired_state.value() ? "open" : "close") : "toggle");
+
+  const SidePanelEntry::Key extension_key(SidePanelEntry::Id::kExtension,
+                                          extension_id);
+
+  content::WebContents* active_web_contents =
+      browser_window.GetActiveTabInterface()->GetContents();
+  tabs::TabInterface* tab = tabs::TabInterface::GetFromContents(&web_contents);
+  SidePanelRegistry* contextual_registry =
+      tab->GetTabFeatures()->side_panel_registry();
+
+  SidePanelUI* side_panel_ui = browser_window.GetFeatures().side_panel_ui();
+  bool is_active_tab = (&web_contents == active_web_contents);
+
+  // Check if this extension's contextual panel is currently showing.
+  bool is_currently_open = false;
+  if (is_active_tab && side_panel_ui->IsSidePanelShowing()) {
+    is_currently_open = IsKeyActiveInRegistry(contextual_registry, extension_key);
+  }
+
+  LOG(INFO) << "browseros: is_currently_open=" << is_currently_open
+            << ", is_active_tab=" << is_active_tab;
+
+  // Determine what action to take.
+  bool should_open;
+  if (desired_state.has_value()) {
+    should_open = desired_state.value();
+  } else {
+    // Toggle: open if closed, close if open.
+    should_open = !is_currently_open;
+  }
+
+  LOG(INFO) << "browseros: should_open=" << should_open;
+
+  // If already in desired state, return early.
+  if (should_open == is_currently_open) {
+    LOG(INFO) << "browseros: Already in desired state, no action needed";
+    return is_currently_open;
+  }
+
+  if (!should_open) {
+    LOG(INFO) << "browseros: Closing contextual panel";
+    side_panel_ui->Close();
+    contextual_registry->ResetActiveEntryFor(SidePanelEntry::PanelType::kContent);
+    return false;
+  } else {
+    LOG(INFO) << "browseros: Opening contextual panel";
+
+    SidePanelEntry* contextual_entry =
+        contextual_registry->GetEntryForKey(extension_key);
+    LOG(INFO) << "browseros: Got contextual_entry: "
+              << (contextual_entry ? "yes" : "no");
+
+    if (!contextual_entry) {
+      LOG(WARNING) << "browseros: No contextual entry found, cannot open";
+      return false;
+    }
+
+    contextual_registry->SetActiveEntry(contextual_entry);
+
+    if (is_active_tab) {
+      LOG(INFO) << "browseros: Calling side_panel_ui->Show() for active tab";
+      side_panel_ui->Show(extension_key);
+    }
+
+    return true;
+  }
+}
+
 }  // namespace extensions::side_panel_util
