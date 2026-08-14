diff --git a/chrome/browser/extensions/browseros_extension_constants.h b/chrome/browser/extensions/browseros_extension_constants.h
index e42df6740460c0ad22e265f510c220941bb59079..50b6d8e61e1f9ddae430b6bff55740b662eb18d3 100644
--- a/chrome/browser/extensions/browseros_extension_constants.h
+++ b/chrome/browser/extensions/browseros_extension_constants.h
@@ -17,6 +17,10 @@ namespace browseros {
 inline constexpr char kAISidePanelExtensionId[] =
     "djhdjhlnljbjgejbndockeedocneiaei";
 
+// Current BrowserOS agent extension ID.
+inline constexpr char kAgentV2ExtensionId[] =
+    "bflpfmnmnokmjhmgnolecpppdbdophmk";
+
 // Bug Reporter Extension ID
 inline constexpr char kBugReporterExtensionId[] =
     "adlpneommgkgeanpaekgoaolcpncohkf";
@@ -46,6 +50,7 @@ inline constexpr BrowserOSExtensionInfo kBrowserOSExtensions[] = {
     {kAISidePanelExtensionId, "BrowserOS", true, true},
     {kBugReporterExtensionId, "BrowserOS/bug-reporter", true, false},
     {kControllerExtensionId, "BrowserOS/controller", false, false},
+    {kAgentV2ExtensionId, "BrowserOS", false, false},
 };
 
 // Allowlist of BrowserOS extension IDs that are permitted to be installed.
@@ -54,6 +59,7 @@ inline constexpr const char* kAllowedExtensions[] = {
     kBrowserOSExtensions[0].id,
     kBrowserOSExtensions[1].id,
     kBrowserOSExtensions[2].id,
+    kBrowserOSExtensions[3].id,
 };
 
 inline constexpr size_t kBrowserOSExtensionsCount =
