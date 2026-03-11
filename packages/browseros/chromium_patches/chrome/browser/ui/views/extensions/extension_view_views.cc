diff --git a/chrome/browser/ui/views/extensions/extension_view_views.cc b/chrome/browser/ui/views/extensions/extension_view_views.cc
index f24639d796626..8fcf248beb22f 100644
--- a/chrome/browser/ui/views/extensions/extension_view_views.cc
+++ b/chrome/browser/ui/views/extensions/extension_view_views.cc
@@ -9,6 +9,7 @@
 
 #include "base/functional/bind.h"
 #include "build/build_config.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/extension_view_host.h"
 #include "chrome/browser/profiles/profile.h"
 #include "chrome/browser/ui/browser.h"
@@ -147,6 +148,14 @@ void ExtensionViewViews::OnLoaded() {
 
   SetVisible(true);
   ResizeDueToAutoResize(web_contents(), pending_preferred_size_);
+
+  // BrowserOS: Auto-focus side panel after loading. RequestFocus() in
+  // PopulateSidePanel() fires before the RWHV exists, so re-request here.
+  if (host_->extension_host_type() ==
+          extensions::mojom::ViewType::kExtensionSidePanel &&
+      browseros::IsBrowserOSExtension(host_->extension_id())) {
+    RequestFocus();
+  }
 }
 
 ui::Cursor ExtensionViewViews::GetCursor(const ui::MouseEvent& event) {
