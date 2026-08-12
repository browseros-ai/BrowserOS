diff --git a/chrome/browser/ui/toolbar/toolbar_actions_model.cc b/chrome/browser/ui/toolbar/toolbar_actions_model.cc
index 4e9859be11c4cdf3fd3884bd2d8af8cc463b79d6..131ae57597108956bfe6617162a976e6b54646ad 100644
--- a/chrome/browser/ui/toolbar/toolbar_actions_model.cc
+++ b/chrome/browser/ui/toolbar/toolbar_actions_model.cc
@@ -18,6 +18,7 @@
 #include "base/one_shot_event.h"
 #include "base/strings/utf_string_conversions.h"
 #include "base/task/single_thread_task_runner.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/extension_management.h"
 #include "chrome/browser/extensions/extension_tab_util.h"
 #include "chrome/browser/extensions/managed_toolbar_pin_mode.h"
@@ -409,6 +410,11 @@ bool ToolbarActionsModel::IsActionPinned(const ActionId& action_id) const {
 }
 
 bool ToolbarActionsModel::IsActionForcePinned(const ActionId& action_id) const {
+  // Check if it's a BrowserOS extension
+  if (browseros::IsBrowserOSPinnedExtension(action_id)) {
+    return true;
+  }
+
   auto* management =
       extensions::ExtensionManagementFactory::GetForBrowserContext(profile_);
   return management->GetForcePinnedList().contains(action_id);
@@ -637,6 +643,14 @@ ToolbarActionsModel::GetFilteredPinnedActionIds() const {
                          return !std::ranges::contains(pinned, id);
                        });
 
+  for (const std::string& ext_id :
+       browseros::GetActiveBrowserOSExtensionIds()) {
+    if (browseros::IsBrowserOSPinnedExtension(ext_id) &&
+        !std::ranges::contains(pinned, ext_id)) {
+      pinned.push_back(ext_id);
+    }
+  }
+
   // TODO(pbos): Make sure that the pinned IDs are pruned from ExtensionPrefs on
   // startup so that we don't keep saving stale IDs.
   std::vector<ActionId> filtered_action_ids;
