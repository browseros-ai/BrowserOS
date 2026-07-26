diff --git a/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.cc b/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.cc
index f598b6139232d21a089401da367641987def6347..59a36a0a9db2dbfe8e3df8bbe3e520476b157a1f 100644
--- a/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.cc
+++ b/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.cc
@@ -14,10 +14,9 @@
 #include "chrome/browser/ui/ui_features.h"
 #include "chrome/grit/branded_strings.h"
 #include "chrome/grit/generated_resources.h"
+#include "chrome/grit/theme_resources.h"
 #include "components/infobars/content/content_infobar_manager.h"
 #include "components/infobars/core/infobar.h"
-#include "components/omnibox/browser/vector_icons.h"
-#include "components/vector_icons/vector_icons.h"
 #include "ui/base/l10n/l10n_util.h"
 
 #if BUILDFLAG(IS_WIN)
@@ -96,9 +95,8 @@ infobars::InfoBarDelegate::InfoBarIdentifier PinInfoBarDelegate::GetIdentifier()
   return PIN_INFOBAR_DELEGATE;
 }
 
-const gfx::VectorIcon& PinInfoBarDelegate::GetVectorIcon() const {
-  return dark_mode() ? omnibox::kProductChromeRefreshIcon
-                     : vector_icons::kProductRefreshIcon;
+int PinInfoBarDelegate::GetIconId() const {
+  return IDR_PRODUCT_LOGO_16;
 }
 
 std::u16string PinInfoBarDelegate::GetMessageText() const {
