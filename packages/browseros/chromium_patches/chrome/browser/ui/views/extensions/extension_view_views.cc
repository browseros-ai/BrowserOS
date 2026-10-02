diff --git a/chrome/browser/ui/views/extensions/extension_view_views.cc b/chrome/browser/ui/views/extensions/extension_view_views.cc
index 3c515b480a110ad657fcf3819c13f962e21f7f7e..47ecd84867a107c8ad451ed56ae9490fbe3ed7e6 100644
--- a/chrome/browser/ui/views/extensions/extension_view_views.cc
+++ b/chrome/browser/ui/views/extensions/extension_view_views.cc
@@ -6,6 +6,7 @@
 
 #include "base/functional/bind.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/extension_view_host.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/views/extensions/extension_popup.h"
@@ -136,6 +137,14 @@ void ExtensionViewViews::OnLoaded() {
 
   SetVisible(true);
   ResizeDueToAutoResize(web_contents(), pending_preferred_size_);
+
+  // BrowserOS: Auto-focus side panel after loading. RequestFocus() in
+  // PopulateSidePanel() fires before the RWHV exists, so re-request here.
+  if (host_->extension_host_type() ==
+          extensions::mojom::ViewType::kExtensionSidePanel &&
+      browseros::IsActiveBrowserOSExtension(host_->extension_id())) {
+    RequestFocus();
+  }
 }
 
 ui::Cursor ExtensionViewViews::GetCursor(const ui::MouseEvent& event) {
