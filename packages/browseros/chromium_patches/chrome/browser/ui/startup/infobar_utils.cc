diff --git a/chrome/browser/ui/startup/infobar_utils.cc b/chrome/browser/ui/startup/infobar_utils.cc
index 598e361a2988674243dd6ace9cc63de2f74ed1e1..6192225cedac276f5f954f1c5e8bdabb7c264da5 100644
--- a/chrome/browser/ui/startup/infobar_utils.cc
+++ b/chrome/browser/ui/startup/infobar_utils.cc
@@ -253,20 +253,6 @@ void AddInfoBarsIfNecessary(BrowserWindowInterface* browser,
   infobars::ContentInfoBarManager* infobar_manager =
       infobars::ContentInfoBarManager::FromWebContents(web_contents);
 
-  if (!google_apis::HasAPIKeyConfigured()) {
-    if (infobars::IsInfoBarMigrated(
-            infobars::InfoBarDelegate::GOOGLE_API_KEYS_INFOBAR_DELEGATE)) {
-      if (auto* manager =
-              infobars::BrowserInfoBarManager::From(g_browser_process)) {
-        manager->Show(
-            tabs::TabInterface::GetFromContents(web_contents),
-            infobars::InfoBarDelegate::GOOGLE_API_KEYS_INFOBAR_DELEGATE);
-      }
-    } else {
-      GoogleApiKeysInfoBarDelegate::Create(infobar_manager);
-    }
-  }
-
   if (ObsoleteSystem::IsObsoleteNowOrSoon()) {
     PrefService* local_state = g_browser_process->local_state();
     if (!local_state ||
