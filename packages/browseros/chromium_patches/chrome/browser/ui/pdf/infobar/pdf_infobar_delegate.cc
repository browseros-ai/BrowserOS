diff --git a/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc b/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc
index 6a2b16400ffe1358bd790f1b8b8082d0a2a21ab0..d99305cf7fd5ba9337805342690a503e6a277e9c 100644
--- a/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc
+++ b/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc
@@ -12,10 +12,9 @@
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
@@ -45,11 +44,8 @@ infobars::InfoBarDelegate::InfoBarIdentifier PdfInfoBarDelegate::GetIdentifier()
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
