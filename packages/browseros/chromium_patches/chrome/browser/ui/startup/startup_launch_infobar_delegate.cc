diff --git a/chrome/browser/ui/startup/startup_launch_infobar_delegate.cc b/chrome/browser/ui/startup/startup_launch_infobar_delegate.cc
index a8d86170c9a33617c53be2d6edf56bb71272ad28..559f8e31d2653c41868daa9808542dae573773c7 100644
--- a/chrome/browser/ui/startup/startup_launch_infobar_delegate.cc
+++ b/chrome/browser/ui/startup/startup_launch_infobar_delegate.cc
@@ -16,11 +16,10 @@
 #include "chrome/common/webui_url_constants.h"
 #include "chrome/grit/branded_strings.h"
 #include "chrome/grit/generated_resources.h"
+#include "chrome/grit/theme_resources.h"
 #include "components/infobars/core/confirm_infobar_delegate.h"
 #include "components/infobars/core/infobar.h"
-#include "components/omnibox/browser/vector_icons.h"
 #include "components/prefs/pref_service.h"
-#include "components/vector_icons/vector_icons.h"
 #include "ui/base/l10n/l10n_util.h"
 #include "ui/base/ui_base_types.h"
 
@@ -47,9 +46,8 @@ StartupLaunchInfoBarDelegate::GetIdentifier() const {
   return STARTUP_LAUNCH_INFOBAR_DELEGATE;
 }
 
-const gfx::VectorIcon& StartupLaunchInfoBarDelegate::GetVectorIcon() const {
-  return dark_mode() ? omnibox::kProductChromeRefreshIcon
-                     : vector_icons::kProductRefreshIcon;
+int StartupLaunchInfoBarDelegate::GetIconId() const {
+  return IDR_PRODUCT_LOGO_16;
 }
 
 bool StartupLaunchInfoBarDelegate::ShouldExpire(
