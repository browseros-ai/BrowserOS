diff --git a/chrome/browser/sessions/session_service_base.cc b/chrome/browser/sessions/session_service_base.cc
index b443ebe6862a1b5024c5f171ace575fa69de902f..47679905b4dd7b9a3ffd862de5a59df7419607ae 100644
--- a/chrome/browser/sessions/session_service_base.cc
+++ b/chrome/browser/sessions/session_service_base.cc
@@ -823,6 +823,11 @@ bool SessionServiceBase::ShouldTrackBrowser(
     return false;
   }
 
+  // Hidden Browsers are ephemeral agent workspaces; never persist them.
+  if (browser->GetBrowserForMigrationOnly()->is_hidden()) {
+    return false;
+  }
+
   // Never track app popup windows that do not have a trusted source (i.e.
   // popup windows spawned by an app). If this logic changes, be sure to also
   // change SessionRestoreImpl::CreateRestoredBrowser().
