diff --git a/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc b/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc
index 35297a7f2e9ef..ca411a035a81d 100644
--- a/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc
+++ b/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc
@@ -882,6 +882,7 @@ class RemoveDownloadsTester {
   raw_ptr<ChromeDownloadManagerDelegate> chrome_download_manager_delegate_;
 };
 
+#if BUILDFLAG(ENABLE_REPORTING)
 base::RepeatingCallback<bool(const GURL&)> CreateUrlFilterFromOriginFilter(
     const base::RepeatingCallback<bool(const url::Origin&)>& origin_filter) {
   if (origin_filter.is_null()) {
@@ -891,6 +892,7 @@ base::RepeatingCallback<bool(const GURL&)> CreateUrlFilterFromOriginFilter(
     return origin_filter.Run(url::Origin::Create(url));
   });
 }
+#endif  // BUILDFLAG(ENABLE_REPORTING)
 
 class RemoveAutofillTester {
  public:
