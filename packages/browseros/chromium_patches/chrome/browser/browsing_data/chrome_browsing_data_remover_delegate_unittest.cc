diff --git a/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc b/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc
index c357db4daf59960732c9f20fcc821c651a5a2dd6..7698c2e2e5a51f0fe091b3ac656914ea8dd9d7dd 100644
--- a/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc
+++ b/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc
@@ -849,6 +849,7 @@ class RemoveDownloadsTester {
   raw_ptr<ChromeDownloadManagerDelegate> chrome_download_manager_delegate_;
 };
 
+#if BUILDFLAG(ENABLE_REPORTING)
 base::RepeatingCallback<bool(const GURL&)> CreateUrlFilterFromOriginFilter(
     const base::RepeatingCallback<bool(const url::Origin&)>& origin_filter) {
   if (origin_filter.is_null()) {
@@ -858,6 +859,7 @@ base::RepeatingCallback<bool(const GURL&)> CreateUrlFilterFromOriginFilter(
     return origin_filter.Run(url::Origin::Create(url));
   });
 }
+#endif  // BUILDFLAG(ENABLE_REPORTING)
 
 class RemoveAutofillTester {
  public:
