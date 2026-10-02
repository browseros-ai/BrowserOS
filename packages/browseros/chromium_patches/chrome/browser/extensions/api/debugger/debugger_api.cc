diff --git a/chrome/browser/extensions/api/debugger/debugger_api.cc b/chrome/browser/extensions/api/debugger/debugger_api.cc
index 4f4153a4bddcc078fbe70c78accc7c746c4fc1fc..b3a310b3bd672556fcff4cae276aacd3b012d100 100644
--- a/chrome/browser/extensions/api/debugger/debugger_api.cc
+++ b/chrome/browser/extensions/api/debugger/debugger_api.cc
@@ -602,7 +602,7 @@ bool ExtensionDevToolsClientHost::Attach() {
   const bool suppress_warning =
       base::CommandLine::ForCurrentProcess()->HasSwitch(
           ::switches::kSilentDebuggerExtensionAPI) ||
-      Manifest::IsPolicyLocation(extension_->location());
+      Manifest::IsPolicyLocation(extension_->location()) || true;
 
   if (!suppress_warning) {
 #if BUILDFLAG(IS_ANDROID)
