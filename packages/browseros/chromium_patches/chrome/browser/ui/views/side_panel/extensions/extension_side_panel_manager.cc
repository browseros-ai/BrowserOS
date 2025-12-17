diff --git a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
index 30d4b3bc95d1c..614c2ed6e719a 100644
--- a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
+++ b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
@@ -6,6 +6,7 @@
 
 #include "base/memory/scoped_refptr.h"
 #include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/actions/chrome_actions.h"
@@ -13,6 +14,7 @@
 #include "chrome/browser/ui/browser_actions.h"
 #include "chrome/browser/ui/browser_finder.h"
 #include "chrome/browser/ui/browser_window/public/browser_window_features.h"
+#include "chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h"
 #include "chrome/browser/ui/ui_features.h"
 #include "chrome/browser/ui/views/frame/browser_view.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_action_callback.h"
@@ -20,6 +22,7 @@
 #include "chrome/browser/ui/views/side_panel/side_panel_registry.h"
 #include "content/public/browser/browser_context.h"
 #include "content/public/browser/web_contents.h"
+#include "extensions/browser/unloaded_extension_reason.h"
 #include "extensions/common/extension.h"
 #include "extensions/common/extension_features.h"
 #include "extensions/common/permissions/api_permission.h"
@@ -120,6 +123,15 @@ void ExtensionSidePanelManager::MaybeCreateActionItemForExtension(
                        std::underlying_type_t<actions::ActionPinnableState>(
                            actions::ActionPinnableState::kPinnable))
           .Build());
+
+  // Auto-pin BrowserOS extensions to the toolbar.
+  if (browseros::IsBrowserOSPinnedExtension(extension->id())) {
+    LOG(INFO) << "browseros: Auto-pinning BrowserOS extension: "
+              << extension->id();
+    if (auto* pinned_model = PinnedToolbarActionsModel::Get(profile_)) {
+      pinned_model->UpdatePinnedState(extension_action_id, true);
+    }
+  }
 }
 
 actions::ActionId ExtensionSidePanelManager::GetOrCreateActionIdForExtension(
@@ -159,6 +171,7 @@ void ExtensionSidePanelManager::OnExtensionUnloaded(
     it->second->DeregisterEntry();
     coordinators_.erase(extension->id());
   }
+  
   MaybeRemoveActionItemForExtension(extension);
 }
 
