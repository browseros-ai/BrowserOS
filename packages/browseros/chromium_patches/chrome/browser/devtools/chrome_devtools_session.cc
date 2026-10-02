diff --git a/chrome/browser/devtools/chrome_devtools_session.cc b/chrome/browser/devtools/chrome_devtools_session.cc
index a93a5d26c146eaa90f89294a804bf74ace3439de..fd4bc1e515ae50b1dab0021bb0e371359f6021fa 100644
--- a/chrome/browser/devtools/chrome_devtools_session.cc
+++ b/chrome/browser/devtools/chrome_devtools_session.cc
@@ -15,10 +15,12 @@
 #include "chrome/browser/devtools/features.h"
 #include "chrome/browser/devtools/protocol/ads_handler.h"
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
         std::make_unique<BrowserHandler>(dispatcher(), agent_host->GetId());
   }
+  if (IsDomainAvailableToUntrustedClient<BookmarksHandler>() ||
+      channel->GetClient()->IsTrusted()) {
+    bookmarks_handler_ =
+        std::make_unique<BookmarksHandler>(dispatcher(), agent_host->GetId());
+  }
+  if (IsDomainAvailableToUntrustedClient<HistoryHandler>() ||
+      channel->GetClient()->IsTrusted()) {
+    history_handler_ =
+        std::make_unique<HistoryHandler>(dispatcher(), agent_host->GetId());
+  }
   if (IsDomainAvailableToUntrustedClient<SystemInfoHandler>() ||
       channel->GetClient()->IsTrusted()) {
     system_info_handler_ = std::make_unique<SystemInfoHandler>(dispatcher());
