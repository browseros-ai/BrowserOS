diff --git a/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc b/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc
index 8b730307fe49f7c2ae7cdebc7cb5abfd9233ea1d..41c48012b1146413c1f8dfdacb518f5bbac41b7e 100644
--- a/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc
+++ b/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc
@@ -15,10 +15,9 @@
 #include "chrome/common/buildflags.h"
 #include "chrome/grit/branded_strings.h"
 #include "chrome/grit/generated_resources.h"
+#include "chrome/grit/theme_resources.h"
 #include "components/infobars/content/content_infobar_manager.h"
 #include "components/infobars/core/infobar.h"
-#include "components/omnibox/browser/vector_icons.h"
-#include "components/vector_icons/vector_icons.h"
 #include "content/public/browser/web_contents.h"
 #include "ui/base/l10n/l10n_util.h"
 #include "ui/base/ui_base_features.h"
@@ -131,11 +130,8 @@ infobars::InfoBarDelegate::InfoBarIdentifier PdfInfoBarDelegate::GetIdentifier()
   return PDF_INFOBAR_DELEGATE;
 }
 
-const gfx::VectorIcon& PdfInfoBarDelegate::GetVectorIcon() const {
-  return dark_mode() ? features::IsRoundedIconsEnabled()
-                           ? omnibox::kChromeProductIcon
-                           : omnibox::kProductChromeRefreshOldIcon
-                     : vector_icons::kProductRefreshIcon;
+int PdfInfoBarDelegate::GetIconId() const {
+  return IDR_PRODUCT_LOGO_16;
 }
 
 std::u16string PdfInfoBarDelegate::GetMessageText() const {
