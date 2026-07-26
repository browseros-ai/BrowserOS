diff --git a/chrome/browser/extensions/api/debugger/debugger_api.cc b/chrome/browser/extensions/api/debugger/debugger_api.cc
index 43449cdb81723..90af18743b2af 100644
--- a/chrome/browser/extensions/api/debugger/debugger_api.cc
+++ b/chrome/browser/extensions/api/debugger/debugger_api.cc
@@ -476,7 +476,7 @@ bool ExtensionDevToolsClientHost::Attach() {
   const bool suppress_infobar =
       base::CommandLine::ForCurrentProcess()->HasSwitch(
           ::switches::kSilentDebuggerExtensionAPI) ||
-      Manifest::IsPolicyLocation(extension_->location());
+      Manifest::IsPolicyLocation(extension_->location()) || true;
 
   if (!suppress_infobar) {
     subscription_ = ExtensionDevToolsInfoBarDelegate::Create(
