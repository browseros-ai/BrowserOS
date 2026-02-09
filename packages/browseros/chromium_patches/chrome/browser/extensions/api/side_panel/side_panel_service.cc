diff --git a/chrome/browser/extensions/api/side_panel/side_panel_service.cc b/chrome/browser/extensions/api/side_panel/side_panel_service.cc
index b4e8c1ce7316c..c6ba8b2fc1ca6 100644
--- a/chrome/browser/extensions/api/side_panel/side_panel_service.cc
+++ b/chrome/browser/extensions/api/side_panel/side_panel_service.cc
@@ -8,9 +8,11 @@
 #include <memory>
 #include <optional>
 
+#include "base/logging.h"
 #include "base/no_destructor.h"
 #include "base/strings/stringprintf.h"
 #include "base/strings/to_string.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/extension_tab_util.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
@@ -471,6 +473,139 @@ void SidePanelService::OnExtensionUninstalled(
   RemoveExtensionOptions(extension->id());
 }
 
+base::expected<bool, std::string>
+SidePanelService::BrowserosToggleSidePanelForTab(
+    const Extension& extension,
+    content::BrowserContext* context,
+    int tab_id,
+    bool include_incognito_information,
+    std::optional<bool> desired_state) {
+  LOG(INFO) << "browseros: BrowserosToggleSidePanelForTab called for tab_id="
+            << tab_id << ", extension=" << extension.id()
+            << ", desired_state="
+            << (desired_state.has_value()
+                    ? (desired_state.value() ? "open" : "close")
+                    : "toggle");
+
+  // Find the tab.
+  WindowController* window = nullptr;
+  content::WebContents* web_contents = nullptr;
+  if (!ExtensionTabUtil::GetTabById(tab_id, context,
+                                    include_incognito_information, &window,
+                                    &web_contents, nullptr) ||
+      !window || !web_contents) {
+    LOG(WARNING) << "browseros: Tab not found for tab_id=" << tab_id;
+    return base::unexpected(ErrorUtils::FormatErrorMessage(
+        ExtensionTabUtil::kTabNotFoundError, base::ToString(tab_id)));
+  }
+
+  BrowserWindowInterface* browser_window = window->GetBrowserWindowInterface();
+  if (!browser_window) {
+    LOG(WARNING) << "browseros: No browser window for tab_id=" << tab_id;
+    return base::unexpected(
+        base::StringPrintf("No browser window for tabId: %d", tab_id));
+  }
+
+  // Auto-register contextual panel options if none exist for this tab.
+  // This ensures the panel is tab-specific (contextual) and won't bleed to
+  // other tabs.
+  auto panels_iter = panels_.find(extension.id());
+  bool has_contextual_options = false;
+  if (panels_iter != panels_.end()) {
+    has_contextual_options = panels_iter->second.contains(tab_id);
+  }
+
+  LOG(INFO) << "browseros: has_contextual_options=" << has_contextual_options
+            << " for tab_id=" << tab_id;
+
+  if (!has_contextual_options) {
+    // Get the default/manifest path to use for this contextual panel.
+    api::side_panel::PanelOptions default_options =
+        GetOptions(extension, std::nullopt);
+    if (!default_options.path) {
+      LOG(WARNING) << "browseros: No side panel path configured for extension="
+                   << extension.id();
+      return base::unexpected(
+          "No side panel path configured. Set a path in manifest or via "
+          "setOptions() before toggling.");
+    }
+
+    LOG(INFO) << "browseros: Auto-registering contextual panel for tab_id="
+              << tab_id << " with path=" << *default_options.path;
+
+    // Create contextual options for this tab.
+    api::side_panel::PanelOptions contextual_options;
+    contextual_options.tab_id = tab_id;
+    contextual_options.path = std::move(default_options.path);
+    contextual_options.enabled = true;
+    SetOptions(extension, std::move(contextual_options));
+  }
+
+  // Check if panel is disabled for this tab.
+  api::side_panel::PanelOptions current_options = GetOptions(extension, tab_id);
+  if (!current_options.enabled.value_or(true)) {
+    LOG(WARNING) << "browseros: Side panel is disabled for tab_id=" << tab_id;
+    return base::unexpected(
+        base::StringPrintf("Side panel is disabled for tabId: %d", tab_id));
+  }
+
+  // Toggle the contextual panel.
+  LOG(INFO) << "browseros: Calling ToggleContextualExtensionSidePanel for tab_id="
+            << tab_id;
+  bool is_now_open = side_panel_util::ToggleContextualExtensionSidePanel(
+      *browser_window, *web_contents, extension.id(), desired_state);
+
+  LOG(INFO) << "browseros: Toggle result: is_now_open=" << is_now_open
+            << " for tab_id=" << tab_id;
+
+  return is_now_open;
+}
+
+base::expected<bool, std::string>
+SidePanelService::BrowserosIsSidePanelOpenForTab(
+    const Extension& extension,
+    content::BrowserContext* context,
+    int tab_id,
+    bool include_incognito_information) {
+  LOG(INFO) << "browseros: BrowserosIsSidePanelOpenForTab called for tab_id="
+            << tab_id << ", extension=" << extension.id();
+
+  // Find the tab.
+  WindowController* window = nullptr;
+  content::WebContents* web_contents = nullptr;
+  if (!ExtensionTabUtil::GetTabById(tab_id, context,
+                                    include_incognito_information, &window,
+                                    &web_contents, nullptr) ||
+      !window || !web_contents) {
+    LOG(WARNING) << "browseros: Tab not found for tab_id=" << tab_id;
+    return base::unexpected(ErrorUtils::FormatErrorMessage(
+        ExtensionTabUtil::kTabNotFoundError, base::ToString(tab_id)));
+  }
+
+  BrowserWindowInterface* browser_window = window->GetBrowserWindowInterface();
+  if (!browser_window) {
+    LOG(WARNING) << "browseros: No browser window for tab_id=" << tab_id;
+    return base::unexpected(
+        base::StringPrintf("No browser window for tabId: %d", tab_id));
+  }
+
+  // Check if panel is disabled - return false (not an error).
+  api::side_panel::PanelOptions current_options = GetOptions(extension, tab_id);
+  if (!current_options.enabled.value_or(true)) {
+    LOG(INFO) << "browseros: Panel is disabled for tab_id=" << tab_id
+              << ", returning false";
+    return false;
+  }
+
+  bool is_open = side_panel_util::IsContextualExtensionSidePanelOpen(
+      browser_window, web_contents, extension.id());
+
+  LOG(INFO) << "browseros: IsOpen result: is_open=" << is_open
+            << " for tab_id=" << tab_id;
+
+  return is_open;
+}
+
 void SidePanelService::Shutdown() {
   for (auto& observer : observers_) {
     observer.OnSidePanelServiceShutdown();
