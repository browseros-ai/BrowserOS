diff --git a/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.cc b/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.cc
index 32a98bbe7bb4be60015f2a3e3888f7933cc49b16..0704970581cffd1b0e8b7f81cc6f93edcd3ef5c4 100644
--- a/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.cc
+++ b/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.cc
@@ -17,14 +17,12 @@
 #include "chrome/common/pref_names.h"
 #include "chrome/grit/branded_strings.h"
 #include "chrome/grit/generated_resources.h"
+#include "chrome/grit/theme_resources.h"
 #include "components/infobars/content/content_infobar_manager.h"
 #include "components/infobars/core/infobar.h"
 #include "components/infobars/core/infobar_manager.h"
-#include "components/omnibox/browser/vector_icons.h"
 #include "components/prefs/pref_service.h"
-#include "components/vector_icons/vector_icons.h"
 #include "ui/base/l10n/l10n_util.h"
-#include "ui/gfx/vector_icon_types.h"
 
 namespace session_restore_infobar {
 
@@ -140,9 +138,8 @@ SessionRestoreInfoBarDelegate::GetIdentifier() const {
       SESSION_RESTORE_INFOBAR_DELEGATE;
 }
 
-const gfx::VectorIcon& SessionRestoreInfoBarDelegate::GetVectorIcon() const {
-  return dark_mode() ? omnibox::kProductChromeRefreshIcon
-                     : vector_icons::kProductRefreshIcon;
+int SessionRestoreInfoBarDelegate::GetIconId() const {
+  return IDR_PRODUCT_LOGO_16;
 }
 
 bool SessionRestoreInfoBarDelegate::ShouldExpire(
