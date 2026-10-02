diff --git a/chrome/browser/ui/extensions/extension_side_panel_utils.cc b/chrome/browser/ui/extensions/extension_side_panel_utils.cc
index aaeb4c24d8353813739b2012e1b719ad7f26fb5e..3c83083367f2432fd152ee2d63295a9c1a87582b 100644
--- a/chrome/browser/ui/extensions/extension_side_panel_utils.cc
+++ b/chrome/browser/ui/extensions/extension_side_panel_utils.cc
@@ -307,4 +307,83 @@ void CloseContextualExtensionSidePanel(BrowserWindowInterface* browser_window,
   contextual_registry->ResetActiveEntry();
 }
 
+// BrowserOS contextual APIs retain per-tab state even when Chromium's standard
+// extension APIs opt into the window-scoped Organizer panel. Use the contextual
+// registry and SidePanelUI together rather than the Organizer-aware helpers.
+bool IsContextualExtensionSidePanelOpen(BrowserWindowInterface* browser_window,
+                                        content::WebContents* web_contents,
+                                        const ExtensionId& extension_id) {
+  if (!browser_window || !web_contents) {
+    return false;
+  }
+
+  const SidePanelEntry::Key extension_key(SidePanelEntry::Id::kExtension,
+                                          extension_id);
+  SidePanelRegistry* contextual_registry = GetTabRegistry(web_contents);
+  if (!IsKeyActiveInRegistry(contextual_registry, extension_key)) {
+    return false;
+  }
+
+  tabs::TabInterface* active_tab = GetActiveTab(browser_window);
+  if (active_tab && active_tab->GetContents() == web_contents) {
+    SidePanelUI* side_panel_ui = SidePanelUI::From(browser_window);
+    return side_panel_ui &&
+           side_panel_ui->IsSidePanelEntryShowing(extension_key,
+                                                  /*for_tab=*/true);
+  }
+
+  // A background tab remembers its open entry while another tab owns the
+  // visible window panel. That remembered state is what toggle must invert.
+  return true;
+}
+
+bool ToggleContextualExtensionSidePanel(BrowserWindowInterface& browser_window,
+                                        content::WebContents& web_contents,
+                                        const ExtensionId& extension_id,
+                                        std::optional<bool> desired_state) {
+  SidePanelRegistry* contextual_registry = GetTabRegistry(&web_contents);
+  if (!contextual_registry) {
+    return false;
+  }
+
+  const SidePanelEntry::Key extension_key(SidePanelEntry::Id::kExtension,
+                                          extension_id);
+  tabs::TabInterface* active_tab = GetActiveTab(&browser_window);
+  const bool is_active_tab =
+      active_tab && active_tab->GetContents() == &web_contents;
+  SidePanelUI* side_panel_ui = SidePanelUI::From(&browser_window);
+  const bool is_currently_open = IsContextualExtensionSidePanelOpen(
+      &browser_window, &web_contents, extension_id);
+  const bool should_open = desired_state.value_or(!is_currently_open);
+  if (should_open == is_currently_open) {
+    return is_currently_open;
+  }
+
+  if (!should_open) {
+    // Closing a background tab's remembered panel must not close the active
+    // tab's UI. Avoid the standard helper, whose Organizer path is window-wide.
+    if (is_active_tab && side_panel_ui &&
+        side_panel_ui->IsSidePanelEntryShowing(extension_key,
+                                               /*for_tab=*/true)) {
+      side_panel_ui->Close();
+    }
+    // Clear remembered state immediately, including during the close animation,
+    // so switching tabs cannot restore a panel that the caller just closed.
+    contextual_registry->ResetActiveEntry();
+    return false;
+  }
+
+  SidePanelEntry* contextual_entry =
+      contextual_registry->GetEntryForKey(extension_key);
+  if (!contextual_entry || (is_active_tab && !side_panel_ui)) {
+    return false;
+  }
+
+  contextual_registry->SetActiveEntry(contextual_entry);
+  if (is_active_tab) {
+    side_panel_ui->Show(extension_key);
+  }
+  return true;
+}
+
 }  // namespace extensions::side_panel_util
