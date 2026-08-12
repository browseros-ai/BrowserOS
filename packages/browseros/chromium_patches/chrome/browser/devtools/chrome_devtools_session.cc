diff --git a/chrome/browser/devtools/chrome_devtools_session.cc b/chrome/browser/devtools/chrome_devtools_session.cc
index d79c1e2c50439fe7edf6f4deab17f0373f431776..3675913f661e328f87ef26235eeb47070d696675 100644
--- a/chrome/browser/devtools/chrome_devtools_session.cc
+++ b/chrome/browser/devtools/chrome_devtools_session.cc
@@ -18,10 +18,12 @@
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
@@ -125,6 +127,16 @@ ChromeDevToolsSession::ChromeDevToolsSession(
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
