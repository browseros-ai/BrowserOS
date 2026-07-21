diff --git a/chrome/browser/ui/toasts/api/toast_id.cc b/chrome/browser/ui/toasts/api/toast_id.cc
index f167d2e1c1ceeb44c0890c38f82b14b401f57ee2..cfac472d3863149a67f9cb6b5dc6be97fc97a44c 100644
--- a/chrome/browser/ui/toasts/api/toast_id.cc
+++ b/chrome/browser/ui/toasts/api/toast_id.cc
@@ -101,6 +101,8 @@ std::string_view GetToastName(ToastId toast_id) {
       return "AutofillAiPreFetchErrorMessage";
     case ToastId::kDictationError:
       return "DictationError";
+    case ToastId::kBrowserOSToast:
+      return "BrowserOSToast";
   }
 
   NOTREACHED();
