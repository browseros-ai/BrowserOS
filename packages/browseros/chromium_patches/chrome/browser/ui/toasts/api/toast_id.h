diff --git a/chrome/browser/ui/toasts/api/toast_id.h b/chrome/browser/ui/toasts/api/toast_id.h
index 06abc76b800df44f2278f290134a3a81939e61f0..5e320273d3ea5730ec0031c8b23156b484aa361a 100644
--- a/chrome/browser/ui/toasts/api/toast_id.h
+++ b/chrome/browser/ui/toasts/api/toast_id.h
@@ -67,7 +67,8 @@ enum class ToastId {
   kTabStripSwitchDelayedVertical = 44,
   kAutofillAiPreFetchErrorMessage = 45,
   kDictationError = 48,
-  kMaxValue = kDictationError,
+  kBrowserOSToast = 49,
+  kMaxValue = kBrowserOSToast,
 };
 // LINT.ThenChange(/tools/metrics/histograms/metadata/toasts/enums.xml:ToastId)
 
