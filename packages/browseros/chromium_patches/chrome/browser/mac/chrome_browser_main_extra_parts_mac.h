diff --git a/chrome/browser/mac/chrome_browser_main_extra_parts_mac.h b/chrome/browser/mac/chrome_browser_main_extra_parts_mac.h
index 95726e7765367..da1f407aff2b3 100644
--- a/chrome/browser/mac/chrome_browser_main_extra_parts_mac.h
+++ b/chrome/browser/mac/chrome_browser_main_extra_parts_mac.h
@@ -24,6 +24,7 @@ class ChromeBrowserMainExtraPartsMac : public ChromeBrowserMainExtraParts {
 
   // ChromeBrowserMainExtraParts:
   void PreEarlyInitialization() override;
+  void PreCreateMainMessageLoop() override;
 
  private:
   std::unique_ptr<display::ScopedNativeScreen> screen_;
