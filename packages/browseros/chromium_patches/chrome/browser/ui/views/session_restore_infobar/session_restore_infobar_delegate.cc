diff --git a/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.cc b/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.cc
index 75ffc714223b2443ae89e08cd6d8418d932c864b..e54794ba9d7afe41749c5321b5b85c60bbb81a25 100644
--- a/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.cc
+++ b/chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_delegate.cc
@@ -16,16 +16,14 @@
 #include "chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_prefs.h"
 #include "chrome/common/pref_names.h"
 #include "chrome/grit/branded_strings.h"
+#include "chrome/grit/generated_resources.h"
+#include "chrome/grit/theme_resources.h"
 #include "components/infobars/content/content_infobar_manager.h"
 #include "components/infobars/core/infobar.h"
 #include "components/infobars/core/infobar_manager.h"
-#include "components/omnibox/browser/vector_icons.h"
 #include "components/prefs/pref_service.h"
-#include "components/vector_icons/vector_icons.h"
 #include "ui/base/l10n/l10n_util.h"
-#include "ui/base/ui_base_features.h"
 #include "ui/base/window_open_disposition.h"
-#include "ui/gfx/vector_icon_types.h"
 
 namespace session_restore_infobar {
 
@@ -97,11 +95,8 @@ SessionRestoreInfoBarDelegate::GetIdentifier() const {
       SESSION_RESTORE_INFOBAR_DELEGATE;
 }
 
-const gfx::VectorIcon& SessionRestoreInfoBarDelegate::GetVectorIcon() const {
-  return dark_mode() ? features::IsRoundedIconsEnabled()
-                           ? omnibox::kChromeProductIcon
-                           : omnibox::kProductChromeRefreshOldIcon
-                     : vector_icons::kProductRefreshIcon;
+int SessionRestoreInfoBarDelegate::GetIconId() const {
+  return IDR_PRODUCT_LOGO_16;
 }
 
 bool SessionRestoreInfoBarDelegate::ShouldExpire(
