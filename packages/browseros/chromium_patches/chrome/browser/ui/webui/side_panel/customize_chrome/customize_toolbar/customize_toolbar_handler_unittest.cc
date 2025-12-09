diff --git a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
index 44e84bea67455..d3ee987cad870 100644
--- a/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
+++ b/chrome/browser/ui/webui/side_panel/customize_chrome/customize_toolbar/customize_toolbar_handler_unittest.cc
@@ -290,7 +290,7 @@ TEST_F(CustomizeToolbarHandlerTest, PinForward) {
 }
 
 TEST_F(CustomizeToolbarHandlerTest, PinSplitTab) {
-  ASSERT_FALSE(profile()->GetPrefs()->GetBoolean(prefs::kPinSplitTabButton));
+  ASSERT_TRUE(profile()->GetPrefs()->GetBoolean(prefs::kPinSplitTabButton));
 
   handler().PinAction(side_panel::customize_chrome::mojom::ActionId::kSplitTab,
                       false);
