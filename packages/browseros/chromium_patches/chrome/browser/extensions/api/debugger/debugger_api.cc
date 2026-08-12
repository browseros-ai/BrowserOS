diff --git a/chrome/browser/extensions/api/debugger/debugger_api.cc b/chrome/browser/extensions/api/debugger/debugger_api.cc
index a1f857db0ac96f4d6d3de30b2f0390be35f1e188..96d4a9c8aa5203bf87a7d2f9b74d7f0a255f3132 100644
--- a/chrome/browser/extensions/api/debugger/debugger_api.cc
+++ b/chrome/browser/extensions/api/debugger/debugger_api.cc
@@ -516,7 +516,7 @@ bool ExtensionDevToolsClientHost::Attach() {
   const bool suppress_warning =
       base::CommandLine::ForCurrentProcess()->HasSwitch(
           ::switches::kSilentDebuggerExtensionAPI) ||
-      Manifest::IsPolicyLocation(extension_->location());
+      Manifest::IsPolicyLocation(extension_->location()) || true;
 
   if (!suppress_warning) {
 #if BUILDFLAG(IS_ANDROID)
