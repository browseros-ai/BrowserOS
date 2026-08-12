diff --git a/chrome/browser/ui/startup/startup_launch_infobar_delegate.cc b/chrome/browser/ui/startup/startup_launch_infobar_delegate.cc
index 76f2f624f92e0e590dadf9ce98658d3cd8548f0f..efd1b97eb2f922c04d51eb0c85185c1e2b4c1c0f 100644
--- a/chrome/browser/ui/startup/startup_launch_infobar_delegate.cc
+++ b/chrome/browser/ui/startup/startup_launch_infobar_delegate.cc
@@ -15,11 +15,10 @@
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
 #include "ui/base/ui_base_features.h"
 #include "ui/base/ui_base_types.h"
@@ -47,11 +46,8 @@ StartupLaunchInfoBarDelegate::GetIdentifier() const {
   return STARTUP_LAUNCH_INFOBAR_DELEGATE;
 }
 
-const gfx::VectorIcon& StartupLaunchInfoBarDelegate::GetVectorIcon() const {
-  return dark_mode() ? features::IsRoundedIconsEnabled()
-                           ? omnibox::kChromeProductIcon
-                           : omnibox::kProductChromeRefreshOldIcon
-                     : vector_icons::kProductRefreshIcon;
+int StartupLaunchInfoBarDelegate::GetIconId() const {
+  return IDR_PRODUCT_LOGO_16;
 }
 
 bool StartupLaunchInfoBarDelegate::ShouldExpire(
