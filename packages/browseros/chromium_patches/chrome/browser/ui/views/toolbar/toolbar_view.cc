diff --git a/chrome/browser/ui/views/toolbar/toolbar_view.cc b/chrome/browser/ui/views/toolbar/toolbar_view.cc
index bfc981b7ee720c039cd927c5cfc717ed72fa1bd5..b23e3a6312756bc25888849275eb1ae4b6668729 100644
--- a/chrome/browser/ui/views/toolbar/toolbar_view.cc
+++ b/chrome/browser/ui/views/toolbar/toolbar_view.cc
@@ -25,6 +25,7 @@
 #include "chrome/browser/actor/ui/actor_ui_metrics.h"
 #include "chrome/browser/actor/ui/task_list_bubble/actor_task_list_bubble.h"
 #include "chrome/browser/actor/ui/task_list_bubble/actor_task_list_bubble_controller.h"
+#include "chrome/browser/browseros/customization/customization_pref_names.h"
 #include "chrome/browser/command_updater.h"
 #include "chrome/browser/glic/browser_ui/glic_actor_nudge_controller.h"
 #include "chrome/browser/glic/browser_ui/glic_actor_task_icon_manager_factory.h"
@@ -409,7 +410,14 @@ void ToolbarView::Init() {
   // profile (only regular and incognito profiles host extensions).
   if (!browser_->GetProfile()->IsGuestSession() &&
       !features::IsWebUIExtensionsContainerEnabled()) {
-    extensions_container = std::make_unique<ExtensionsToolbarDesktop>(browser_);
+    // BrowserOS: with browseros.show_extensions_toolbar off, e.g. forced by
+    // --disable-browseros-features=ExtensionsToolbar, use the auto-hide mode of
+    // web-app windows. The area then appears only while an extension popup or
+    // the extensions menu needs it, so popups keep their anchor.
+    extensions_container = std::make_unique<ExtensionsToolbarDesktop>(
+        browser_, prefs->GetBoolean(browseros::prefs::kShowExtensionsToolbar)
+                      ? ExtensionsToolbarDesktop::DisplayMode::kNormal
+                      : ExtensionsToolbarDesktop::DisplayMode::kAutoHide);
 
     toolbar_divider = std::make_unique<ToolbarDivider>();
   }
@@ -598,6 +606,12 @@ void ToolbarView::Init() {
     avatar_ = AddChildView(std::make_unique<AvatarToolbarButton>(browser_));
     bool show_avatar_toolbar_button =
         AvatarToolbarButtonInterface::CanShowForProfile(browser_->GetProfile());
+    // BrowserOS: browseros.show_avatar_button can hide it, e.g. forced off by
+    // --disable-browseros-features=AvatarButton. Nothing changes the avatar's
+    // visibility after this, so reading the pref once per window is enough.
+    if (!prefs->GetBoolean(browseros::prefs::kShowAvatarButton)) {
+      show_avatar_toolbar_button = false;
+    }
     avatar_->SetVisible(show_avatar_toolbar_button);
   }
 
