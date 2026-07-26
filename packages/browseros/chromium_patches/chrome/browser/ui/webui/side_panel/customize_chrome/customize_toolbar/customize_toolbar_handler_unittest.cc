diff --git a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
index 20a546a4bf150..764c9a5f74cfa 100644
--- a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
+++ b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
@@ -298,7 +298,7 @@ TEST_F(CustomizeToolbarHandlerTest, PinForward) {
 }
 
 TEST_F(CustomizeToolbarHandlerTest, PinSplitTab) {
-  ASSERT_FALSE(profile()->GetPrefs()->GetBoolean(prefs::kPinSplitTabButton));
+  ASSERT_TRUE(profile()->GetPrefs()->GetBoolean(prefs::kPinSplitTabButton));
 
   handler().PinAction(side_panel::customize_chrome::mojom::ActionId::kSplitTab,
                       false);
