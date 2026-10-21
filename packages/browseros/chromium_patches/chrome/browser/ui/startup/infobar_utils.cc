diff --git a/chrome/browser/ui/startup/infobar_utils.cc b/chrome/browser/ui/startup/infobar_utils.cc
index 598e361a2988674243dd6ace9cc63de2f74ed1e1..83ddc9e218440a245644a89e924ae67f4bda25f1 100644
--- a/chrome/browser/ui/startup/infobar_utils.cc
+++ b/chrome/browser/ui/startup/infobar_utils.cc
@@ -17,6 +17,7 @@
 #include "build/branding_buildflags.h"
 #include "build/buildflag.h"
 #include "chrome/browser/browser_process.h"
+#include "chrome/browser/browseros/core/browseros_product.h"
 #include "chrome/browser/infobars/browser_infobar_manager.h"
 #include "chrome/browser/infobars/infobar_features.h"
 #include "chrome/browser/obsolete_system/obsolete_system.h"
@@ -253,20 +254,6 @@ void AddInfoBarsIfNecessary(BrowserWindowInterface* browser,
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
@@ -320,7 +307,8 @@ void AddInfoBarsIfNecessary(BrowserWindowInterface* browser,
 #endif  // BUILDFLAG(IS_MAC) || BUILDFLAG(IS_WIN) || BUILDFLAG(IS_LINUX)
 
   // The default browser prompt should only be shown after the first run.
-  if (is_first_run == chrome::startup::IsFirstRun::kNo) {
+  if (is_first_run == chrome::startup::IsFirstRun::kNo &&
+      !browseros::IsBrowserClawProduct()) {
 #if BUILDFLAG(IS_MAC) || BUILDFLAG(IS_WIN)
     if (base::FeatureList::IsEnabled(features::kSeparateDefaultAndPinPrompt)) {
       const int seed = features::kSeparateDefaultAndPinPromptRandSeed.Get();
