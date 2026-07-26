diff --git a/chrome/browser/devtools/chrome_devtools_session.h b/chrome/browser/devtools/chrome_devtools_session.h
index bac525f420dee..52d94ea97d9ee 100644
--- a/chrome/browser/devtools/chrome_devtools_session.h
+++ b/chrome/browser/devtools/chrome_devtools_session.h
@@ -21,10 +21,12 @@ class DevToolsAgentHostClientChannel;
 }  // namespace content
 
 class AutofillHandler;
+class BookmarksHandler;
 class EmulationHandler;
 class BrowserHandler;
 class CastHandler;
 class PageHandler;
+class HistoryHandler;
 class PWAHandler;
 class SecurityHandler;
 class StorageHandler;
@@ -66,10 +68,12 @@ class ChromeDevToolsSession : public protocol::FrontendChannel {
 
   protocol::UberDispatcher dispatcher_;
   std::unique_ptr<AutofillHandler> autofill_handler_;
+  std::unique_ptr<BookmarksHandler> bookmarks_handler_;
   std::unique_ptr<ExtensionsHandler> extensions_handler_;
   std::unique_ptr<BrowserHandler> browser_handler_;
   std::unique_ptr<CastHandler> cast_handler_;
   std::unique_ptr<EmulationHandler> emulation_handler_;
+  std::unique_ptr<HistoryHandler> history_handler_;
   std::unique_ptr<PageHandler> page_handler_;
   std::unique_ptr<PWAHandler> pwa_handler_;
   std::unique_ptr<SecurityHandler> security_handler_;
