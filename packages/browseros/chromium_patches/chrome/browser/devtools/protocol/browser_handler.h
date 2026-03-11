diff --git a/chrome/browser/devtools/protocol/browser_handler.h b/chrome/browser/devtools/protocol/browser_handler.h
index e1424aa52cbf6..ffd1c86c5aed9 100644
--- a/chrome/browser/devtools/protocol/browser_handler.h
+++ b/chrome/browser/devtools/protocol/browser_handler.h
@@ -5,9 +5,17 @@
 #ifndef CHROME_BROWSER_DEVTOOLS_PROTOCOL_BROWSER_HANDLER_H_
 #define CHROME_BROWSER_DEVTOOLS_PROTOCOL_BROWSER_HANDLER_H_
 
+#include <memory>
+
+#include "base/containers/flat_map.h"
 #include "base/containers/flat_set.h"
+#include "base/memory/raw_ptr.h"
 #include "chrome/browser/devtools/protocol/browser.h"
 
+class Browser;
+class BrowserWindowInterface;
+class Profile;
+
 class BrowserHandler : public protocol::Browser::Backend {
  public:
   BrowserHandler(protocol::UberDispatcher* dispatcher,
@@ -23,6 +31,14 @@ class BrowserHandler : public protocol::Browser::Backend {
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
@@ -41,9 +57,118 @@ class BrowserHandler : public protocol::Browser::Backend {
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
+  protocol::Response ShowWindow(int window_id) override;
+  protocol::Response HideWindow(int window_id) override;
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
+      std::optional<bool> hidden,
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
+  protocol::Response HideTab(
+      std::optional<std::string> target_id,
+      std::optional<int> tab_id,
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
+  Browser* GetOrCreateHiddenWindow(Profile* profile);
+  void MakeWindowHidden(Browser* browser);
+  void MakeWindowVisible(BrowserWindowInterface* bwi);
+  bool IsHiddenWindow(int window_id) const;
+
   base::flat_set<std::string> contexts_with_overridden_permissions_;
   std::string target_id_;
+  base::flat_set<int> hidden_window_ids_;
+  base::flat_map<raw_ptr<Profile>, raw_ptr<Browser>> hidden_window_per_profile_;
 };
 
 #endif  // CHROME_BROWSER_DEVTOOLS_PROTOCOL_BROWSER_HANDLER_H_
