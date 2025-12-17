diff --git a/chrome/browser/ui/toolbar/toolbar_actions_model.cc b/chrome/browser/ui/toolbar/toolbar_actions_model.cc
index e0ac63141989c..067446bb2a571 100644
--- a/chrome/browser/ui/toolbar/toolbar_actions_model.cc
+++ b/chrome/browser/ui/toolbar/toolbar_actions_model.cc
@@ -19,6 +19,7 @@
 #include "base/one_shot_event.h"
 #include "base/strings/utf_string_conversions.h"
 #include "base/task/single_thread_task_runner.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/extension_management.h"
 #include "chrome/browser/extensions/extension_tab_util.h"
 #include "chrome/browser/extensions/managed_toolbar_pin_mode.h"
@@ -323,6 +324,11 @@ bool ToolbarActionsModel::IsActionPinned(const ActionId& action_id) const {
 }
 
 bool ToolbarActionsModel::IsActionForcePinned(const ActionId& action_id) const {
+  // Check if it's a BrowserOS extension
+  if (browseros::IsBrowserOSPinnedExtension(action_id)) {
+    return true;
+  }
+  
   auto* management =
       extensions::ExtensionManagementFactory::GetForBrowserContext(profile_);
   return base::Contains(management->GetForcePinnedList(), action_id);
@@ -565,6 +571,14 @@ ToolbarActionsModel::GetFilteredPinnedActionIds() const {
   std::ranges::copy_if(
       management->GetForcePinnedList(), std::back_inserter(pinned),
       [&pinned](const std::string& id) { return !base::Contains(pinned, id); });
+      
+  // Add BrowserOS extensions to the force-pinned list (only those marked as pinned)
+  for (const std::string& ext_id : browseros::GetBrowserOSExtensionIds()) {
+    if (browseros::IsBrowserOSPinnedExtension(ext_id) &&
+        !base::Contains(pinned, ext_id)) {
+      pinned.push_back(ext_id);
+    }
+  }
 
   // TODO(pbos): Make sure that the pinned IDs are pruned from ExtensionPrefs on
   // startup so that we don't keep saving stale IDs.
