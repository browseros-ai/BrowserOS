diff --git a/chrome/browser/extensions/chrome_extensions_browser_api_provider.cc b/chrome/browser/extensions/chrome_extensions_browser_api_provider.cc
index a251aaaa53378..94a89781536b2 100644
--- a/chrome/browser/extensions/chrome_extensions_browser_api_provider.cc
+++ b/chrome/browser/extensions/chrome_extensions_browser_api_provider.cc
@@ -19,7 +19,6 @@ ChromeExtensionsBrowserAPIProvider::~ChromeExtensionsBrowserAPIProvider() =
 
 void ChromeExtensionsBrowserAPIProvider::RegisterExtensionFunctions(
     ExtensionFunctionRegistry* registry) {
-  // Generated APIs from Chrome.
   api::ChromeGeneratedFunctionRegistry::RegisterAll(registry);
 }
 
