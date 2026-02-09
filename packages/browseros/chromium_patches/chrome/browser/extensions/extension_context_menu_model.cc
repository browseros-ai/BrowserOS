diff --git a/chrome/browser/extensions/extension_context_menu_model.cc b/chrome/browser/extensions/extension_context_menu_model.cc
index 7d8d30399b256..bd3356e448d51 100644
--- a/chrome/browser/extensions/extension_context_menu_model.cc
+++ b/chrome/browser/extensions/extension_context_menu_model.cc
@@ -6,6 +6,7 @@
 
 #include <memory>
 
+#include "chrome/browser/browseros/core/browseros_constants.h"
 #include "base/feature_list.h"
 #include "base/functional/bind.h"
 #include "base/metrics/histogram_macros.h"
@@ -806,7 +807,8 @@ void ExtensionContextMenuModel::InitMenuWithFeature(
 
   // Controls section.
   bool has_options_page = OptionsPageInfo::HasOptionsPage(extension);
-  bool can_uninstall_extension = !is_component_ && !is_required_by_policy;
+  bool can_uninstall_extension = !is_component_ && !is_required_by_policy &&
+                                  !browseros::IsBrowserOSExtension(extension->id());
   if (can_show_icon_in_toolbar || has_options_page || can_uninstall_extension) {
     AddSeparator(ui::NORMAL_SEPARATOR);
   }
