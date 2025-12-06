diff --git a/chrome/browser/extensions/api/side_panel/side_panel_service.h b/chrome/browser/extensions/api/side_panel/side_panel_service.h
index 623e81e776d2f..cfe2abc1b2bc7 100644
--- a/chrome/browser/extensions/api/side_panel/side_panel_service.h
+++ b/chrome/browser/extensions/api/side_panel/side_panel_service.h
@@ -161,6 +161,26 @@ class SidePanelService : public BrowserContextKeyedAPI,
                              std::optional<int> tab_id,
                              const std::string& path);
 
+  // Toggles the extension's contextual side panel for a specific tab.
+  // If `desired_state` is provided, opens (true) or closes (false) the panel.
+  // If `desired_state` is nullopt, toggles the current state.
+  // Auto-registers contextual panel options if none exist for the tab.
+  // Returns the new state (true = open, false = closed) or an error on failure.
+  base::expected<bool, std::string> BrowserosToggleSidePanelForTab(
+      const Extension& extension,
+      content::BrowserContext* context,
+      int tab_id,
+      bool include_incognito_information,
+      std::optional<bool> desired_state);
+
+  // Checks if the extension's contextual side panel is open for a specific tab.
+  // Returns true if open, false if closed, or an error string on failure.
+  base::expected<bool, std::string> BrowserosIsSidePanelOpenForTab(
+      const Extension& extension,
+      content::BrowserContext* context,
+      int tab_id,
+      bool include_incognito_information);
+
  private:
   friend class BrowserContextKeyedAPIFactory<SidePanelService>;
 
