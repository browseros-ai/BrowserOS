diff --git a/chrome/browser/devtools/chrome_devtools_session.cc b/chrome/browser/devtools/chrome_devtools_session.cc
index a93a5d26c146eaa90f89294a804bf74ace3439de..8945ca89677a65626af99e833ba15b0be2f3dd37 100644
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
@@ -98,6 +100,11 @@ ChromeDevToolsSession::ChromeDevToolsSession(
           dispatcher(), agent_host->GetWebContents());
     }
   }
+  if (agent_host->GetType() == content::DevToolsAgentHost::kTypeBrowser &&
+      channel->GetClient()->IsTrusted()) {
+    autofill_handler_ =
+        std::make_unique<AutofillHandler>(dispatcher(), agent_host->GetId());
+  }
   if (IsDomainAvailableToUntrustedClient<ExtensionsHandler>() ||
       channel->GetClient()->IsTrusted()) {
     extensions_handler_ = std::make_unique<ExtensionsHandler>(
@@ -120,6 +127,16 @@ ChromeDevToolsSession::ChromeDevToolsSession(
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
