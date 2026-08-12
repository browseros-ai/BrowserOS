diff --git a/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.cc b/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.cc
index 34ee63e9ca0026959f5f16d90b85c82755eef52c..3dd0ee066376136fdc22ceb2f8862b8fc35f337c 100644
--- a/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.cc
+++ b/chrome/browser/ui/startup/default_browser_prompt/pin_infobar/pin_infobar_delegate.cc
@@ -13,10 +13,10 @@
 #include "chrome/browser/infobars/confirm_infobar_creator.h"
 #include "chrome/browser/ui/ui_features.h"
 #include "chrome/grit/branded_strings.h"
+#include "chrome/grit/generated_resources.h"
+#include "chrome/grit/theme_resources.h"
 #include "components/infobars/content/content_infobar_manager.h"
 #include "components/infobars/core/infobar.h"
-#include "components/omnibox/browser/vector_icons.h"
-#include "components/vector_icons/vector_icons.h"
 #include "ui/base/l10n/l10n_util.h"
 #include "ui/base/ui_base_features.h"
 
@@ -96,11 +96,8 @@ infobars::InfoBarDelegate::InfoBarIdentifier PinInfoBarDelegate::GetIdentifier()
   return PIN_INFOBAR_DELEGATE;
 }
 
-const gfx::VectorIcon& PinInfoBarDelegate::GetVectorIcon() const {
-  return dark_mode() ? features::IsRoundedIconsEnabled()
-                           ? omnibox::kChromeProductIcon
-                           : omnibox::kProductChromeRefreshOldIcon
-                     : vector_icons::kProductRefreshIcon;
+int PinInfoBarDelegate::GetIconId() const {
+  return IDR_PRODUCT_LOGO_16;
 }
 
 std::u16string PinInfoBarDelegate::GetMessageText() const {
