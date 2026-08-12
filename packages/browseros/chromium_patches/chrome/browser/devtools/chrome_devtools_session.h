diff --git a/chrome/browser/devtools/chrome_devtools_session.h b/chrome/browser/devtools/chrome_devtools_session.h
index 59232dff240078b5d9d0d1dea7dddff3be512796..9786024c9020906cb3235f3393173c4124971422 100644
--- a/chrome/browser/devtools/chrome_devtools_session.h
+++ b/chrome/browser/devtools/chrome_devtools_session.h
@@ -22,10 +22,12 @@ class DevToolsAgentHostClientChannel;
 
 class AdsHandler;
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
@@ -61,10 +63,12 @@ class ChromeDevToolsSession : public protocol::FrontendChannel {
   protocol::UberDispatcher dispatcher_;
   std::unique_ptr<AdsHandler> ads_handler_;
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
