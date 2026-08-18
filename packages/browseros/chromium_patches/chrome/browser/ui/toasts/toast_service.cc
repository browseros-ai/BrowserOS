diff --git a/chrome/browser/ui/toasts/toast_service.cc b/chrome/browser/ui/toasts/toast_service.cc
index 7a4edd62433c3fdd679858610d3671fd484764a1..f049651f7c6287ab65fb56d42e5f162e529e8f54 100644
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
