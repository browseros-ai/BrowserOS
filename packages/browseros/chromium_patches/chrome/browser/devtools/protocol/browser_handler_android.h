diff --git a/chrome/browser/devtools/protocol/browser_handler_android.h b/chrome/browser/devtools/protocol/browser_handler_android.h
index a80686d439110..b35c043695542 100644
--- a/chrome/browser/devtools/protocol/browser_handler_android.h
+++ b/chrome/browser/devtools/protocol/browser_handler_android.h
@@ -22,6 +22,14 @@ class BrowserHandlerAndroid : public protocol::Browser::Backend {
       std::optional<std::string> target_id,
       int* out_window_id,
       std::unique_ptr<protocol::Browser::Bounds>* out_bounds) override;
+  protocol::Response GetTabForTarget(
+      std::optional<std::string> target_id,
+      int* out_tab_id,
+      int* out_window_id) override;
+  protocol::Response GetTargetForTab(
+      int tab_id,
+      std::string* out_target_id,
+      int* out_window_id) override;
   protocol::Response GetWindowBounds(
       int window_id,
       std::unique_ptr<protocol::Browser::Bounds>* out_bounds) override;
@@ -40,6 +48,108 @@ class BrowserHandlerAndroid : public protocol::Browser::Backend {
   protocol::Response AddPrivacySandboxEnrollmentOverride(
       const std::string& in_url) override;
 
+  // Window management
+  protocol::Response GetWindows(
+      std::unique_ptr<protocol::Array<protocol::Browser::WindowInfo>>*
+          out_windows) override;
+  protocol::Response GetActiveWindow(
+      std::unique_ptr<protocol::Browser::WindowInfo>* out_window) override;
+  protocol::Response CreateWindow(
+      std::optional<std::string> url,
+      std::unique_ptr<protocol::Browser::Bounds> bounds,
+      std::optional<std::string> window_type,
+      std::optional<bool> hidden,
+      std::optional<std::string> browser_context_id,
+      std::unique_ptr<protocol::Browser::WindowInfo>* out_window) override;
+  protocol::Response CloseWindow(int window_id) override;
+  protocol::Response ActivateWindow(int window_id) override;
+  protocol::Response SetWindowVisibility(
+      int window_id,
+      bool visible,
+      std::optional<bool> activate,
+      std::unique_ptr<protocol::Browser::WindowInfo>* out_window,
+      bool* out_replaced,
+      int* out_previous_window_id) override;
+
+  // Tab management
+  protocol::Response GetTabs(
+      std::optional<int> window_id,
+      std::optional<bool> include_hidden,
+      std::unique_ptr<protocol::Array<protocol::Browser::TabInfo>>* out_tabs)
+      override;
+  protocol::Response GetActiveTab(
+      std::optional<int> window_id,
+      std::unique_ptr<protocol::Browser::TabInfo>* out_tab) override;
+  protocol::Response GetTabInfo(
+      std::optional<std::string> target_id,
+      std::optional<int> tab_id,
+      std::unique_ptr<protocol::Browser::TabInfo>* out_tab) override;
+  protocol::Response CreateTab(
+      std::optional<std::string> url,
+      std::optional<int> window_id,
+      std::optional<int> index,
+      std::optional<bool> background,
+      std::optional<bool> pinned,
+      std::optional<std::string> browser_context_id,
+      std::unique_ptr<protocol::Browser::TabInfo>* out_tab) override;
+  protocol::Response CloseTab(std::optional<std::string> target_id,
+                              std::optional<int> tab_id) override;
+  protocol::Response ActivateTab(std::optional<std::string> target_id,
+                                 std::optional<int> tab_id) override;
+  protocol::Response MoveTab(
+      std::optional<std::string> target_id,
+      std::optional<int> tab_id,
+      std::optional<int> window_id,
+      std::optional<int> index,
+      std::unique_ptr<protocol::Browser::TabInfo>* out_tab) override;
+  protocol::Response DuplicateTab(
+      std::optional<std::string> target_id,
+      std::optional<int> tab_id,
+      std::unique_ptr<protocol::Browser::TabInfo>* out_tab) override;
+  protocol::Response PinTab(
+      std::optional<std::string> target_id,
+      std::optional<int> tab_id,
+      std::unique_ptr<protocol::Browser::TabInfo>* out_tab) override;
+  protocol::Response UnpinTab(
+      std::optional<std::string> target_id,
+      std::optional<int> tab_id,
+      std::unique_ptr<protocol::Browser::TabInfo>* out_tab) override;
+  protocol::Response ShowTab(
+      std::optional<std::string> target_id,
+      std::optional<int> tab_id,
+      std::optional<int> window_id,
+      std::optional<int> index,
+      std::optional<bool> activate,
+      std::unique_ptr<protocol::Browser::TabInfo>* out_tab) override;
+
+  // Tab group management
+  protocol::Response GetTabGroups(
+      std::optional<int> window_id,
+      std::unique_ptr<protocol::Array<protocol::Browser::TabGroupInfo>>*
+          out_groups) override;
+  protocol::Response CreateTabGroup(
+      std::unique_ptr<protocol::Array<int>> tab_ids,
+      std::optional<std::string> title,
+      std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) override;
+  protocol::Response UpdateTabGroup(
+      const std::string& group_id,
+      std::optional<std::string> title,
+      std::optional<std::string> color,
+      std::optional<bool> collapsed,
+      std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) override;
+  protocol::Response CloseTabGroup(const std::string& group_id) override;
+  protocol::Response AddTabsToGroup(
+      const std::string& group_id,
+      std::unique_ptr<protocol::Array<int>> tab_ids,
+      std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) override;
+  protocol::Response RemoveTabsFromGroup(
+      std::unique_ptr<protocol::Array<int>> tab_ids) override;
+  protocol::Response MoveTabGroup(
+      const std::string& group_id,
+      std::optional<int> window_id,
+      std::optional<int> index,
+      std::unique_ptr<protocol::Browser::TabGroupInfo>* out_group) override;
+
  private:
   const std::string target_id_;
 };
