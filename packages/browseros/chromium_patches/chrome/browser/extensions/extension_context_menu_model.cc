diff --git a/chrome/browser/extensions/extension_context_menu_model.cc b/chrome/browser/extensions/extension_context_menu_model.cc
index 1e239298fab827300c4f1645a2d588e48413ddbb..7010a3050d7370e1390f0db65df2a66dd23eddcd 100644
--- a/chrome/browser/extensions/extension_context_menu_model.cc
+++ b/chrome/browser/extensions/extension_context_menu_model.cc
@@ -15,6 +15,7 @@
 #include "base/notreached.h"
 #include "base/strings/utf_string_conversions.h"
 #include "base/types/pass_key.h"
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "chrome/browser/extensions/context_menu_matcher.h"
 #include "chrome/browser/extensions/extension_management.h"
 #include "chrome/browser/extensions/extension_tab_util.h"
@@ -859,7 +860,11 @@ void ExtensionContextMenuModel::InitMenuWithFeature(
 
   // Controls section.
   bool has_options_page = OptionsPageInfo::HasOptionsPage(extension);
-  bool can_uninstall_extension = !is_component_ && !is_required_by_policy;
+  // Product-managed extensions are maintained by BrowserOS, so the toolbar
+  // menu must not offer removal alongside ordinary user-installed extensions.
+  bool can_uninstall_extension =
+      !is_component_ && !is_required_by_policy &&
+      !browseros::IsActiveBrowserOSExtension(extension->id());
   if (can_show_icon_in_toolbar || has_options_page || can_uninstall_extension) {
     AddSeparator(ui::NORMAL_SEPARATOR);
   }
