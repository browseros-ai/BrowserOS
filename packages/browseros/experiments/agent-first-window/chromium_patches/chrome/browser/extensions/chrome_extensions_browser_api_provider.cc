diff --git a/chrome/browser/extensions/chrome_extensions_browser_api_provider.cc b/chrome/browser/extensions/chrome_extensions_browser_api_provider.cc
index 3666bf5a0d2c845c0060444a8ffa33c017a0c69b..d0f992e8adea4a126115218eb3c55ba13d66c9d4 100644
--- a/chrome/browser/extensions/chrome_extensions_browser_api_provider.cc
+++ b/chrome/browser/extensions/chrome_extensions_browser_api_provider.cc
@@ -29,6 +29,7 @@ void ChromeExtensionsBrowserAPIProvider::RegisterExtensionFunctions(
   registry->RegisterFunction<api::BrowserOSInputTextFunction>();
   registry->RegisterFunction<api::BrowserOSClearFunction>();
   registry->RegisterFunction<api::BrowserOSExecuteJavaScriptFunction>();
+  registry->RegisterFunction<api::BrowserOSToggleAgentSplitFunction>();
 
   // Generated APIs from Chrome.
   api::ChromeGeneratedFunctionRegistry::RegisterAll(registry);
