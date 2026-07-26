diff --git a/chrome/browser/ui/startup/infobar_utils.cc b/chrome/browser/ui/startup/infobar_utils.cc
index 505e8669edbbf..88201c009146e 100644
--- a/chrome/browser/ui/startup/infobar_utils.cc
+++ b/chrome/browser/ui/startup/infobar_utils.cc
@@ -184,10 +184,6 @@ void AddInfoBarsIfNecessary(BrowserWindowInterface* browser,
   infobars::ContentInfoBarManager* infobar_manager =
       infobars::ContentInfoBarManager::FromWebContents(web_contents);
 
-  if (!google_apis::HasAPIKeyConfigured()) {
-    GoogleApiKeysInfoBarDelegate::Create(infobar_manager);
-  }
-
   if (ObsoleteSystem::IsObsoleteNowOrSoon()) {
     PrefService* local_state = g_browser_process->local_state();
     if (!local_state ||
