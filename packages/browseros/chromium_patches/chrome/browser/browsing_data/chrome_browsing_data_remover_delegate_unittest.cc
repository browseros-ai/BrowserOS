diff --git a/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc b/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc
index d48567325f29813127166b34c596214abbb12885..8f10fdb4f283c2f4adaa0892f9f1dd155a347777 100644
--- a/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc
+++ b/chrome/browser/browsing_data/chrome_browsing_data_remover_delegate_unittest.cc
@@ -828,6 +828,7 @@ class RemoveDownloadsTester {
   raw_ptr<ChromeDownloadManagerDelegate> chrome_download_manager_delegate_;
 };
 
+#if BUILDFLAG(ENABLE_REPORTING)
 base::RepeatingCallback<bool(const GURL&)> CreateUrlFilterFromOriginFilter(
     const base::RepeatingCallback<bool(const url::Origin&)>& origin_filter) {
   if (origin_filter.is_null()) {
@@ -837,6 +838,7 @@ base::RepeatingCallback<bool(const GURL&)> CreateUrlFilterFromOriginFilter(
     return origin_filter.Run(url::Origin::Create(url));
   });
 }
+#endif  // BUILDFLAG(ENABLE_REPORTING)
 
 class RemoveAutofillTester {
  public:
