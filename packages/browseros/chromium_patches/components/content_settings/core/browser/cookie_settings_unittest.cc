diff --git a/components/content_settings/core/browser/cookie_settings_unittest.cc b/components/content_settings/core/browser/cookie_settings_unittest.cc
index c45edc6d2c1cf69fcd4f65e155ddad6733517b82..b2ceee0be9c9169c1c89688d37d23fd18d779936 100644
--- a/components/content_settings/core/browser/cookie_settings_unittest.cc
+++ b/components/content_settings/core/browser/cookie_settings_unittest.cc
@@ -611,6 +611,8 @@ TEST_P(CookieSettingsTestP, CookiesBlockThirdParty) {
 }
 
 TEST_F(CookieSettingsTest, CookiesControlsDefault) {
+  EXPECT_EQ(static_cast<int>(CookieControlsMode::kIncognitoOnly),
+            prefs_.GetInteger(prefs::kCookieControlsMode));
   EXPECT_TRUE(cookie_settings_->IsFullCookieAccessAllowed(
       kBlockedSite, kFirstPartySiteForCookies,
       /*top_frame_origin=*/std::nullopt, net::CookieSettingOverrides(),
