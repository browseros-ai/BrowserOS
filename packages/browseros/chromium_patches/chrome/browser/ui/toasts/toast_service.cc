diff --git a/chrome/browser/ui/toasts/toast_service.cc b/chrome/browser/ui/toasts/toast_service.cc
index 1e85dafdc5a9d4e3a69de3a05a596dde8cf3fa95..161bfdd5a4b2aa509b163fb2740a76f49742840b 100644
--- a/chrome/browser/ui/toasts/toast_service.cc
+++ b/chrome/browser/ui/toasts/toast_service.cc
@@ -425,6 +425,13 @@ void ToastService::RegisterToasts(
           features::IsRoundedIconsEnabled() ? kInfoIcon : kInfoOldIcon)
           .Build());
 
+  // BrowserOS extension toast. The body text is supplied dynamically at show
+  // time via ToastParams::body_string_override, so the spec has no body string
+  // id. Global-scoped so it survives tab switches while it is visible.
+  toast_registry_->RegisterToast(
+      ToastId::kBrowserOSToast,
+      ToastSpecification::Builder(kInfoIcon).AddGlobalScoped().Build());
+
   toast_registry_->RegisterToast(
       ToastId::kAutoSignIn,
       ToastSpecification::Builder(features::IsRoundedIconsEnabled()
