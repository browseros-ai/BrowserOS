diff --git a/chrome/browser/devtools/chrome_devtools_session.cc b/chrome/browser/devtools/chrome_devtools_session.cc
index 0756036495f99..65f21fb911f66 100644
--- a/chrome/browser/devtools/chrome_devtools_session.cc
+++ b/chrome/browser/devtools/chrome_devtools_session.cc
@@ -17,10 +17,12 @@
 #include "chrome/browser/browser_features.h"
 #include "chrome/browser/devtools/features.h"
 #include "chrome/browser/devtools/protocol/autofill_handler.h"
+#include "chrome/browser/devtools/protocol/bookmarks_handler.h"
 #include "chrome/browser/devtools/protocol/browser_handler.h"
 #include "chrome/browser/devtools/protocol/cast_handler.h"
 #include "chrome/browser/devtools/protocol/emulation_handler.h"
 #include "chrome/browser/devtools/protocol/extensions_handler.h"
+#include "chrome/browser/devtools/protocol/history_handler.h"
 #include "chrome/browser/devtools/protocol/page_handler.h"
 #include "chrome/browser/devtools/protocol/pwa_handler.h"
 #include "chrome/browser/devtools/protocol/security_handler.h"
@@ -120,6 +122,16 @@ ChromeDevToolsSession::ChromeDevToolsSession(
     browser_handler_ =
         std::make_unique<BrowserHandler>(&dispatcher_, agent_host->GetId());
   }
+  if (IsDomainAvailableToUntrustedClient<BookmarksHandler>() ||
+      channel->GetClient()->IsTrusted()) {
+    bookmarks_handler_ =
+        std::make_unique<BookmarksHandler>(&dispatcher_, agent_host->GetId());
+  }
+  if (IsDomainAvailableToUntrustedClient<HistoryHandler>() ||
+      channel->GetClient()->IsTrusted()) {
+    history_handler_ =
+        std::make_unique<HistoryHandler>(&dispatcher_, agent_host->GetId());
+  }
   if (IsDomainAvailableToUntrustedClient<SystemInfoHandler>() ||
       channel->GetClient()->IsTrusted()) {
     system_info_handler_ = std::make_unique<SystemInfoHandler>(&dispatcher_);
