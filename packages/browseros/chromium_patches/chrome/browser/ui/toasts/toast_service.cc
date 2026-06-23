diff --git a/chrome/browser/ui/toasts/toast_service.cc b/chrome/browser/ui/toasts/toast_service.cc
index 09f145e630ca2..8084a7da406a8 100644
--- a/chrome/browser/ui/toasts/toast_service.cc
+++ b/chrome/browser/ui/toasts/toast_service.cc
@@ -365,6 +365,13 @@ void ToastService::RegisterToasts(
   toast_registry_->RegisterToast(
       ToastId::kRecordReplay, ToastSpecification::Builder(kInfoIcon).Build());
 
+  // BrowserOS extension toast. The body text is supplied dynamically at show
+  // time via ToastParams::body_string_override, so the spec has no body string
+  // id. Global-scoped so it survives tab switches while it is visible.
+  toast_registry_->RegisterToast(
+      ToastId::kBrowserOSToast,
+      ToastSpecification::Builder(kInfoIcon).AddGlobalScoped().Build());
+
   toast_registry_->RegisterToast(
       ToastId::kAutoSignIn,
       ToastSpecification::Builder(vector_icons::kPasswordManagerIcon,
