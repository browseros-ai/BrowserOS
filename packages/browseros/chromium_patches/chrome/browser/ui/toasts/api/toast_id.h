diff --git a/chrome/browser/ui/toasts/api/toast_id.h b/chrome/browser/ui/toasts/api/toast_id.h
index 2b96636a36c374db16eb5ad9c6a28eaa3465640c..1ece55b8bee03c348604844e4c0b8f00634d1cbd 100644
--- a/chrome/browser/ui/toasts/api/toast_id.h
+++ b/chrome/browser/ui/toasts/api/toast_id.h
@@ -68,7 +68,8 @@ enum class ToastId {
   kAutofillAiPreFetchErrorMessage = 45,
   kDictationError = 48,
   kDictationStopped = 49,
-  kMaxValue = kDictationStopped,
+  kBrowserOSToast = 50,
+  kMaxValue = kBrowserOSToast,
 };
 // LINT.ThenChange(/tools/metrics/histograms/metadata/toasts/enums.xml:ToastId)
 
