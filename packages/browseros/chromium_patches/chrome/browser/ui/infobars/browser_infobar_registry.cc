diff --git a/chrome/browser/ui/infobars/browser_infobar_registry.cc b/chrome/browser/ui/infobars/browser_infobar_registry.cc
index 1e83612fa63a4985595945242e82d307cf943082..900b00d37438dad96b8d9474455fddb90844865f 100644
--- a/chrome/browser/ui/infobars/browser_infobar_registry.cc
+++ b/chrome/browser/ui/infobars/browser_infobar_registry.cc
@@ -51,12 +51,12 @@
 
 #if BUILDFLAG(IS_WIN) || BUILDFLAG(IS_MAC) || BUILDFLAG(IS_LINUX)
 #include "chrome/browser/ui/views/session_restore_infobar/session_restore_infobar_manager.h"
+#include "chrome/grit/theme_resources.h"
 #endif
 
 #if BUILDFLAG(IS_MAC)
 #include "chrome/browser/updater/updater.h"
 #include "chrome/common/pref_names.h"
-#include "chrome/grit/theme_resources.h"
 #endif
 
 namespace infobars {
@@ -345,22 +345,21 @@ void RegisterInfoBars() {
     auto spec =
         InfoBarSpec::Builder(InfoBarDelegate::SESSION_RESTORE_INFOBAR_DELEGATE)
             .SetMessageTextTemplate(u"$1")
-            .SetSubstitutionsCallback(base::BindRepeating(
-                [](content::WebContents*) {
+            .SetSubstitutionsCallback(
+                base::BindRepeating([](content::WebContents*) {
                   return session_restore_infobar::
                       SessionRestoreInfoBarManager::GetInstance()
                           ->GetMessageSubstitutions();
                 }))
             .SetLinkText(l10n_util::GetStringUTF16(IDS_SESSION_RESTORE_LINK))
             .SetLinkNavigationUrl(GURL("chrome://settings/onStartup"))
-            .SetIcon(vector_icons::kProductRefreshIcon)
-            .SetDarkModeIcon(features::IsRoundedIconsEnabled()
-                                 ? omnibox::kChromeProductIcon
-                                 : omnibox::kProductChromeRefreshOldIcon)
+            // This path bypasses SessionRestoreInfoBarDelegate, so select
+            // the same BrowserOS bitmap explicitly for both color modes.
+            .SetIconId(IDR_PRODUCT_LOGO_16)
             .SetScope(InfoBarScope::kGlobal)
             .SetExpireOnNavigation(false)
-            .SetBrowserFilter(base::BindRepeating(
-                [](BrowserWindowInterface* browser) {
+            .SetBrowserFilter(
+                base::BindRepeating([](BrowserWindowInterface* browser) {
                   return session_restore_infobar::
                       SessionRestoreInfoBarManager::GetInstance()
                           ->ShouldTrackBrowser(browser);
