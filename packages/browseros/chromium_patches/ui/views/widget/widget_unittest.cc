diff --git a/ui/views/widget/widget_unittest.cc b/ui/views/widget/widget_unittest.cc
index 7d5098e1473847a5c335a8686ae937fe08ccb4aa..f2105ebec87fb286805acfab8aa49182b6dd8fb5 100644
--- a/ui/views/widget/widget_unittest.cc
+++ b/ui/views/widget/widget_unittest.cc
@@ -265,6 +265,14 @@ TEST_F(WidgetTest, WidgetInitParams) {
   EXPECT_EQ(Widget::InitParams::WindowOpacity::kInferred, init1.opacity);
 }
 
+TEST_F(WidgetTest, HeadlessInitParamDefaultsFalse) {
+  Widget::InitParams params(Widget::InitParams::CLIENT_OWNS_WIDGET,
+                            Widget::InitParams::TYPE_WINDOW);
+  EXPECT_FALSE(params.headless);
+  params.headless = true;
+  EXPECT_TRUE(params.headless);
+}
+
 // Tests that the internal name is propagated through widget initialization to
 // the native widget and back.
 class WidgetWithCustomParamsTest : public WidgetTest {
