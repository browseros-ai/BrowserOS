diff --git a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
index c2437bfc295a9..4b2971f34e108 100644
--- a/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
+++ b/chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.cc
@@ -4,8 +4,10 @@
 
 #include "chrome/browser/ui/views/side_panel/extensions/extension_side_panel_manager.h"
 
+#include "base/logging.h"
 #include "base/memory/scoped_refptr.h"
 #include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/browseros/core/browseros_prefs.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/actions/chrome_action_id.h"
 #include "chrome/browser/ui/actions/chrome_actions.h"
@@ -15,11 +17,13 @@
 #include "chrome/browser/ui/browser_window/public/browser_window_features.h"
 #include "chrome/browser/ui/side_panel/side_panel_action_callback.h"
 #include "chrome/browser/ui/side_panel/side_panel_registry.h"
+#include "chrome/browser/ui/toolbar/pinned_toolbar/pinned_toolbar_actions_model.h"
 #include "chrome/browser/ui/ui_features.h"
 #include "chrome/browser/ui/views/frame/browser_view.h"
 #include "chrome/browser/ui/views/side_panel/side_panel_coordinator.h"
 #include "content/public/browser/browser_context.h"
 #include "content/public/browser/web_contents.h"
+#include "extensions/browser/unloaded_extension_reason.h"
 #include "extensions/common/extension.h"
 #include "extensions/common/extension_features.h"
 #include "extensions/common/permissions/api_permission.h"
@@ -120,6 +124,16 @@ void ExtensionSidePanelManager::MaybeCreateActionItemForExtension(
                        std::underlying_type_t<actions::ActionPinnableState>(
                            actions::ActionPinnableState::kPinnable))
           .Build());
+
+  // Auto-pin BrowserOS extensions to the toolbar.
+  if (browseros::ShouldPinBrowserOSExtension(extension->id(),
+                                             profile_->GetPrefs())) {
+    DVLOG(1) << "browseros: Auto-pinning BrowserOS extension: "
+             << extension->id();
+    if (auto* pinned_model = PinnedToolbarActionsModel::Get(profile_)) {
+      pinned_model->UpdatePinnedState(extension_action_id, true);
+    }
+  }
 }
 
 actions::ActionId ExtensionSidePanelManager::GetOrCreateActionIdForExtension(
@@ -159,6 +173,7 @@ void ExtensionSidePanelManager::OnExtensionUnloaded(
     it->second->DeregisterEntry();
     coordinators_.erase(extension->id());
   }
+
   MaybeRemoveActionItemForExtension(extension);
 }
 
