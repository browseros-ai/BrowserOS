diff --git a/chrome/browser/ui/startup/default_browser_prompt/default_browser_infobar_delegate.cc b/chrome/browser/ui/startup/default_browser_prompt/default_browser_infobar_delegate.cc
index 571ed2313ded67e4cb313eda268495b3ed353723..9591257e8f6a38e652cbc90a62dcebca7ea3c7c4 100644
--- a/chrome/browser/ui/startup/default_browser_prompt/default_browser_infobar_delegate.cc
+++ b/chrome/browser/ui/startup/default_browser_prompt/default_browser_infobar_delegate.cc
@@ -12,10 +12,9 @@
 #include "chrome/browser/ui/startup/default_browser_prompt/default_browser_prompt_prefs.h"
 #include "chrome/grit/branded_strings.h"
 #include "chrome/grit/generated_resources.h"
+#include "chrome/grit/theme_resources.h"
 #include "components/infobars/core/confirm_infobar_delegate.h"
 #include "components/infobars/core/infobar.h"
-#include "components/omnibox/browser/vector_icons.h"
-#include "components/vector_icons/vector_icons.h"
 #include "ui/base/l10n/l10n_util.h"
 
 // static
@@ -42,9 +41,8 @@ DefaultBrowserInfoBarDelegate::GetIdentifier() const {
   return DEFAULT_BROWSER_INFOBAR_DELEGATE;
 }
 
-const gfx::VectorIcon& DefaultBrowserInfoBarDelegate::GetVectorIcon() const {
-  return dark_mode() ? omnibox::kProductChromeRefreshIcon
-                     : vector_icons::kProductRefreshIcon;
+int DefaultBrowserInfoBarDelegate::GetIconId() const {
+  return IDR_PRODUCT_LOGO_16;
 }
 
 bool DefaultBrowserInfoBarDelegate::ShouldExpire(
