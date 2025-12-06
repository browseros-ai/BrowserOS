diff --git a/chrome/browser/ui/views/side_panel/side_panel_action_callback.cc b/chrome/browser/ui/views/side_panel/side_panel_action_callback.cc
index fa5c515c4a6c1..7eb33de899151 100644
--- a/chrome/browser/ui/views/side_panel/side_panel_action_callback.cc
+++ b/chrome/browser/ui/views/side_panel/side_panel_action_callback.cc
@@ -4,10 +4,15 @@
 
 #include "chrome/browser/ui/views/side_panel/side_panel_action_callback.h"
 
+#include "base/logging.h"
+#include "chrome/browser/extensions/api/side_panel/side_panel_service.h"
+#include "chrome/browser/extensions/extension_tab_util.h"
+#include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/browser.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_features.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_ui.h"
+#include "extensions/browser/extension_registry.h"
 
 namespace {
 constexpr std::underlying_type_t<SidePanelOpenTrigger>
@@ -34,3 +39,60 @@ actions::ActionItem::InvokeActionCallback CreateToggleSidePanelActionCallback(
       },
       key, bwi);
 }
+
+actions::ActionItem::InvokeActionCallback
+CreateBrowserosToggleSidePanelActionCallback(
+    const extensions::ExtensionId& extension_id,
+    BrowserWindowInterface* bwi) {
+  return base::BindRepeating(
+      [](extensions::ExtensionId extension_id, BrowserWindowInterface* bwi,
+         actions::ActionItem* item, actions::ActionInvocationContext context) {
+        LOG(INFO) << "browseros: Toolbar action clicked for extension="
+                  << extension_id;
+
+        // Get the active tab.
+        content::WebContents* active_contents =
+            bwi->GetActiveTabInterface()->GetContents();
+        if (!active_contents) {
+          LOG(WARNING) << "browseros: No active tab contents";
+          return;
+        }
+
+        int tab_id = extensions::ExtensionTabUtil::GetTabId(active_contents);
+        LOG(INFO) << "browseros: Active tab_id=" << tab_id;
+
+        // Get the profile and extension.
+        Profile* profile =
+            Profile::FromBrowserContext(active_contents->GetBrowserContext());
+        const extensions::Extension* extension =
+            extensions::ExtensionRegistry::Get(profile)
+                ->enabled_extensions()
+                .GetByID(extension_id);
+
+        if (!extension) {
+          LOG(WARNING) << "browseros: Extension not found: " << extension_id;
+          return;
+        }
+
+        // Use BrowserosToggleSidePanelForTab which auto-registers contextual
+        // options.
+        extensions::SidePanelService* service =
+            extensions::SidePanelService::Get(profile);
+        if (!service) {
+          LOG(WARNING) << "browseros: SidePanelService not found";
+          return;
+        }
+
+        auto result = service->BrowserosToggleSidePanelForTab(
+            *extension, profile, tab_id,
+            /*include_incognito_information=*/true,
+            /*desired_state=*/std::nullopt);
+
+        if (!result.has_value()) {
+          LOG(WARNING) << "browseros: Toggle failed: " << result.error();
+        } else {
+          LOG(INFO) << "browseros: Toggle result: " << result.value();
+        }
+      },
+      extension_id, bwi);
+}
