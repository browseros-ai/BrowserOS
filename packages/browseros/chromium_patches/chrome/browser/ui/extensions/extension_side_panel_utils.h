diff --git a/chrome/browser/ui/extensions/extension_side_panel_utils.h b/chrome/browser/ui/extensions/extension_side_panel_utils.h
index 83f930392f97d..000800fe353c1 100644
--- a/chrome/browser/ui/extensions/extension_side_panel_utils.h
+++ b/chrome/browser/ui/extensions/extension_side_panel_utils.h
@@ -63,6 +63,25 @@ void CloseContextualExtensionSidePanel(BrowserWindowInterface* browser_window,
                                        const ExtensionId& extension_id,
                                        std::optional<int> window_id);
 
+// Returns true if the extension's contextual side panel is currently open
+// (active) for the specified `web_contents`. This checks only the contextual
+// (tab-specific) panel, not global panels.
+// Implemented in extension_side_panel_utils.cc in views/.
+bool IsContextualExtensionSidePanelOpen(BrowserWindowInterface* browser_window,
+                                        content::WebContents* web_contents,
+                                        const ExtensionId& extension_id);
+
+// Toggles the extension's contextual side panel for the specified
+// `web_contents`. If `desired_state` is provided, opens (true) or closes
+// (false) the panel. If `desired_state` is nullopt, toggles the current state.
+// This operates only on contextual (tab-specific) panels. Returns true if the
+// panel is now open, false if closed.
+// Implemented in extension_side_panel_utils.cc in views/.
+bool ToggleContextualExtensionSidePanel(BrowserWindowInterface& browser_window,
+                                        content::WebContents& web_contents,
+                                        const ExtensionId& extension_id,
+                                        std::optional<bool> desired_state);
+
 }  // namespace extensions::side_panel_util
 
 #endif  // CHROME_BROWSER_UI_EXTENSIONS_EXTENSION_SIDE_PANEL_UTILS_H_
