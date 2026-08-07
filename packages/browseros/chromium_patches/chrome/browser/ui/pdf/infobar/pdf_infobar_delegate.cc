diff --git a/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc b/chrome/browser/ui/pdf/infobar/pdf_infobar_delegate.cc
index 3f3868e1ba674ed4680ae36a74589209c3a55c04..5523eaedd37bb4ac74e98e598c058dcd5b5d5fa0 100644
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
 #include "ui/gfx/native_ui_types.h"
@@ -130,9 +129,8 @@ infobars::InfoBarDelegate::InfoBarIdentifier PdfInfoBarDelegate::GetIdentifier()
   return PDF_INFOBAR_DELEGATE;
 }
 
-const gfx::VectorIcon& PdfInfoBarDelegate::GetVectorIcon() const {
-  return dark_mode() ? omnibox::kProductChromeRefreshIcon
-                     : vector_icons::kProductRefreshIcon;
+int PdfInfoBarDelegate::GetIconId() const {
+  return IDR_PRODUCT_LOGO_16;
 }
 
 std::u16string PdfInfoBarDelegate::GetMessageText() const {
