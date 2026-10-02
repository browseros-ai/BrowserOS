diff --git a/chrome/app/chrome_crash_reporter_client.h b/chrome/app/chrome_crash_reporter_client.h
index 7f6390504ccd213c07a87e58288794d304474866..90119daba3479994294ee14785c1d17cb7e34469 100644
--- a/chrome/app/chrome_crash_reporter_client.h
+++ b/chrome/app/chrome_crash_reporter_client.h
@@ -65,6 +65,8 @@ class ChromeCrashReporterClient : public crash_reporter::CrashReporterClient {
   std::vector<base::ReadOnlySharedMemoryRegion>
   GetUserStreamSharedMemoryRegions() override;
 
+  std::string GetUploadUrl() override;
+
  private:
   friend class base::NoDestructor<ChromeCrashReporterClient>;
 
