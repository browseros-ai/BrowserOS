diff --git a/chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_view.cc b/chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_view.cc
new file mode 100644
index 0000000000000..24eefea8b8860
--- /dev/null
+++ b/chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_view.cc
@@ -0,0 +1,474 @@
+// Copyright 2025 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_view.h"
+
+#include <utility>
+
+#include "base/functional/bind.h"
+#include "build/build_config.h"
+#include "chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_coordinator.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
+#include "base/strings/string_number_conversions.h"
+#include "base/strings/stringprintf.h"
+#include "base/strings/utf_string_conversions.h"
+#include "base/timer/timer.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/ui/browser.h"
+#include "chrome/browser/ui/browser_commands.h"
+#include "chrome/browser/ui/browser_navigator.h"
+#include "chrome/browser/ui/browser_navigator_params.h"
+#include "chrome/browser/ui/views/chrome_layout_provider.h"
+#include "components/vector_icons/vector_icons.h"
+#include "content/public/browser/navigation_controller.h"
+#include "content/public/browser/web_contents.h"
+#include "ui/base/l10n/l10n_util.h"
+#include "ui/base/metadata/metadata_impl_macros.h"
+#include "ui/base/models/combobox_model.h"
+#include "ui/base/ui_base_features.h"
+#include "ui/color/color_id.h"
+#include "ui/color/color_provider.h"
+#include "ui/views/background.h"
+#include "ui/views/controls/button/image_button.h"
+#include "ui/views/controls/button/radio_button.h"
+#include "ui/views/controls/combobox/combobox.h"
+#include "ui/views/controls/label.h"
+#include "ui/views/controls/separator.h"
+#include "ui/views/controls/webview/webview.h"
+#include "ui/views/layout/box_layout.h"
+#include "ui/views/layout/flex_layout.h"
+#include "ui/views/vector_icons.h"
+
+namespace {
+
+// ComboboxModel for LLM provider selection
+class LlmProviderComboboxModel : public ui::ComboboxModel {
+ public:
+  explicit LlmProviderComboboxModel(ClashOfGptsCoordinator* coordinator)
+      : coordinator_(coordinator) {}
+  ~LlmProviderComboboxModel() override = default;
+
+  // ui::ComboboxModel:
+  size_t GetItemCount() const override {
+    return coordinator_->GetProviders().size();
+  }
+
+  std::u16string GetItemAt(size_t index) const override {
+    const auto& providers = coordinator_->GetProviders();
+    if (index >= providers.size()) {
+      NOTREACHED();
+    }
+    return providers[index].name;
+  }
+
+ private:
+  raw_ptr<ClashOfGptsCoordinator> coordinator_;
+};
+
+}  // namespace
+
+ClashOfGptsView::ClashOfGptsView(ClashOfGptsCoordinator* coordinator)
+    : coordinator_(coordinator),
+      feedback_timer_(std::make_unique<base::OneShotTimer>()) {
+  // Initialize panes vector based on current pane count
+  panes_.resize(coordinator_->GetPaneCount());
+
+  // Set up the main container with horizontal layout
+  auto* main_layout = SetLayoutManager(std::make_unique<views::FlexLayout>());
+  main_layout->SetOrientation(views::LayoutOrientation::kVertical)
+      .SetMainAxisAlignment(views::LayoutAlignment::kStart)
+      .SetCrossAxisAlignment(views::LayoutAlignment::kStretch);
+
+  // Create header with global controls
+  auto* header = this->AddChildView(std::make_unique<views::View>());
+  header->SetLayoutManager(std::make_unique<views::BoxLayout>(
+      views::BoxLayout::Orientation::kHorizontal,
+      gfx::Insets::TLBR(8, 12, 8, 12),
+      12));  // Increased spacing between elements
+
+  // Add title
+  auto* title_label = header->AddChildView(
+      std::make_unique<views::Label>(u"Clash of GPTs"));
+  title_label->SetFontList(title_label->font_list().Derive(
+      2, gfx::Font::NORMAL, gfx::Font::Weight::MEDIUM));
+
+  // Add spacer
+  auto* spacer = header->AddChildView(std::make_unique<views::View>());
+  static_cast<views::BoxLayout*>(header->GetLayoutManager())
+      ->SetFlexForView(spacer, 1);
+
+  // Add radio buttons for pane count selection
+  auto* pane_count_label = header->AddChildView(
+      std::make_unique<views::Label>(u"Panels:"));
+  pane_count_label->SetEnabledColor(ui::kColorLabelForegroundSecondary);
+
+  one_pane_radio_ = header->AddChildView(
+      std::make_unique<views::RadioButton>(u"1", 1));
+  one_pane_radio_->SetCallback(base::BindRepeating(
+      &ClashOfGptsView::OnPaneCountChanged, base::Unretained(this), 1));
+  one_pane_radio_->SetChecked(coordinator_->GetPaneCount() == 1);
+
+  two_panes_radio_ = header->AddChildView(
+      std::make_unique<views::RadioButton>(u"2", 1));
+  two_panes_radio_->SetCallback(base::BindRepeating(
+      &ClashOfGptsView::OnPaneCountChanged, base::Unretained(this), 2));
+  two_panes_radio_->SetChecked(coordinator_->GetPaneCount() == 2);
+
+  three_panes_radio_ = header->AddChildView(
+      std::make_unique<views::RadioButton>(u"3", 1));
+  three_panes_radio_->SetCallback(base::BindRepeating(
+      &ClashOfGptsView::OnPaneCountChanged, base::Unretained(this), 3));
+  three_panes_radio_->SetChecked(coordinator_->GetPaneCount() == 3);
+
+  // Add some padding before copy button
+  header->AddChildView(std::make_unique<views::View>())
+      ->SetPreferredSize(gfx::Size(16, 0));
+
+  // Add copy content button
+  auto* copy_button = header->AddChildView(
+      std::make_unique<views::ImageButton>(base::BindRepeating(
+          &ClashOfGptsView::OnCopyContent, base::Unretained(this))));
+  copy_button->SetImageModel(
+      views::Button::STATE_NORMAL,
+      ui::ImageModel::FromVectorIcon(vector_icons::kContentCopyIcon, 
+                                    ui::kColorIcon, 20));  // Increased icon size
+  copy_button->SetAccessibleName(u"Copy page content to all panes");
+  copy_button->SetTooltipText(u"Copy main page content to clipboard for all LLMs");
+  copy_button->SetPreferredSize(gfx::Size(32, 32));  // Set button size
+  copy_button->SetImageHorizontalAlignment(views::ImageButton::ALIGN_CENTER);
+  copy_button->SetImageVerticalAlignment(views::ImageButton::ALIGN_MIDDLE);
+
+  // Add feedback label (initially hidden)
+  copy_feedback_label_ = header->AddChildView(
+      std::make_unique<views::Label>(u""));
+  copy_feedback_label_->SetVisible(false);
+  copy_feedback_label_->SetEnabledColor(ui::kColorLabelForegroundSecondary);
+
+  // Add separator
+  AddChildView(std::make_unique<views::Separator>());
+
+  // Create container for the panes
+  panes_container_ = AddChildView(std::make_unique<views::View>());
+  panes_container_->SetProperty(
+      views::kFlexBehaviorKey,
+      views::FlexSpecification(views::MinimumFlexSizeRule::kScaleToZero,
+                               views::MaximumFlexSizeRule::kUnbounded));
+
+  // Create panes based on current count
+  RecreatePanesContainer();
+  // The panes are created in RecreatePanesContainer() above
+
+  // Add footer separator
+  AddChildView(std::make_unique<views::Separator>());
+
+  // Create footer with keyboard shortcuts
+  auto* footer = AddChildView(std::make_unique<views::View>());
+  footer->SetLayoutManager(std::make_unique<views::BoxLayout>(
+      views::BoxLayout::Orientation::kHorizontal,
+      gfx::Insets::TLBR(6, 12, 6, 12),
+      8));
+
+  // Add keyboard shortcuts text
+#if BUILDFLAG(IS_MAC)
+  auto* shortcuts_label = footer->AddChildView(
+      std::make_unique<views::Label>(
+          u"⌨️  Toggle: Cmd+Shift+U  •  Cycle pane: Click dropdown"));
+#else
+  auto* shortcuts_label = footer->AddChildView(
+      std::make_unique<views::Label>(
+          u"⌨️  Toggle: Alt+Shift+U  •  Cycle pane: Click dropdown"));
+#endif
+  shortcuts_label->SetEnabledColor(ui::kColorLabelForegroundSecondary);
+  shortcuts_label->SetFontList(
+      shortcuts_label->font_list().DeriveWithSizeDelta(-1));
+}
+
+ClashOfGptsView::~ClashOfGptsView() {
+  if (feedback_timer_ && feedback_timer_->IsRunning()) {
+    feedback_timer_->Stop();
+  }
+  // No need to clean up WebContents - coordinator owns them and will
+  // clean them up via BrowserListObserver/ProfileObserver
+}
+
+content::WebContents* ClashOfGptsView::GetWebContentsForPane(
+    int pane_index) const {
+  if (pane_index < 0 || pane_index >= static_cast<int>(panes_.size())) {
+    return nullptr;
+  }
+  
+  if (panes_[pane_index].web_view && panes_[pane_index].web_view->web_contents()) {
+    return panes_[pane_index].web_view->web_contents();
+  }
+  
+  return nullptr;
+}
+
+void ClashOfGptsView::NavigatePaneToUrl(int pane_index, const GURL& url) {
+  if (content::WebContents* web_contents = GetWebContentsForPane(pane_index)) {
+    web_contents->GetController().LoadURL(
+        url, content::Referrer(), ui::PAGE_TRANSITION_AUTO_TOPLEVEL, std::string());
+  }
+}
+
+void ClashOfGptsView::ShowCopyFeedback() {
+  if (copy_feedback_label_) {
+    copy_feedback_label_->SetText(u"Content copied to clipboard");
+    copy_feedback_label_->SetVisible(true);
+
+    // Cancel any existing timer
+    if (feedback_timer_->IsRunning()) {
+      feedback_timer_->Stop();
+    }
+
+    // Start timer to hide message after 2.5 seconds
+    feedback_timer_->Start(FROM_HERE, base::Seconds(2.5),
+        base::BindOnce(&ClashOfGptsView::HideFeedbackLabel,
+                       weak_factory_.GetWeakPtr()));
+  }
+}
+
+
+void ClashOfGptsView::OnThemeChanged() {
+  views::View::OnThemeChanged();
+  
+  // Update colors based on theme
+  const auto* color_provider = GetColorProvider();
+  if (!color_provider) {
+    return;
+  }
+  
+  // Set the view background to match the window background
+  SetBackground(views::CreateSolidBackground(
+      color_provider->GetColor(ui::kColorDialogBackground)));
+
+  // Update all labels that use secondary color
+  if (copy_feedback_label_) {
+    copy_feedback_label_->SetEnabledColor(
+        color_provider->GetColor(ui::kColorLabelForegroundSecondary));
+  }
+  
+  // Note: pane_count_label_ doesn't exist - removed this block
+  
+  // Update shortcuts label if it exists
+  auto* footer = panes_container_->parent();
+  if (footer && footer->children().size() > 0) {
+    // Find the shortcuts label (should be one of the last children)
+    for (const auto& child_ptr : footer->children()) {
+      views::View* child = child_ptr.get();
+      if (auto* label = views::AsViewClass<views::Label>(child)) {
+        if (label->GetText().find(u"Toggle:") != std::u16string::npos) {
+          label->SetEnabledColor(
+              color_provider->GetColor(ui::kColorLabelForegroundSecondary));
+        }
+      }
+    }
+  }
+  
+  // Update pane labels with theme colors
+  for (const auto& pane : panes_) {
+    if (pane.pane_label) {
+      pane.pane_label->SetEnabledColor(
+          color_provider->GetColor(ui::kColorLabelForegroundSecondary));
+    }
+    
+    // Force combobox to repaint with new theme
+    if (pane.provider_selector) {
+      pane.provider_selector->SchedulePaint();
+    }
+  }
+  
+  // Force WebViews to update their background
+  for (auto& pane : panes_) {
+    if (pane.web_view) {
+      // WebView background should automatically adapt, but we can force a repaint
+      pane.web_view->SchedulePaint();
+    }
+  }
+}
+
+std::unique_ptr<views::View> ClashOfGptsView::CreatePaneView(int pane_index) {
+  auto pane_container = std::make_unique<views::View>();
+  auto* pane_layout = pane_container->SetLayoutManager(
+      std::make_unique<views::FlexLayout>());
+  pane_layout->SetOrientation(views::LayoutOrientation::kVertical)
+      .SetMainAxisAlignment(views::LayoutAlignment::kStart)
+      .SetCrossAxisAlignment(views::LayoutAlignment::kStretch);
+
+  // Create header for this pane
+  auto* header = pane_container->AddChildView(std::make_unique<views::View>());
+  header->SetLayoutManager(std::make_unique<views::BoxLayout>(
+      views::BoxLayout::Orientation::kHorizontal,
+      gfx::Insets::TLBR(4, 8, 4, 8),
+      12));  // Increased spacing between elements
+
+  // Add pane label
+  std::u16string pane_label_text = u"Pane " + base::NumberToString16(pane_index + 1);
+  panes_[pane_index].pane_label = header->AddChildView(
+      std::make_unique<views::Label>(pane_label_text));
+  panes_[pane_index].pane_label->SetEnabledColor(ui::kColorLabelForegroundSecondary);
+
+  // Add provider dropdown
+  auto provider_model = std::make_unique<LlmProviderComboboxModel>(coordinator_);
+  panes_[pane_index].provider_selector = header->AddChildView(
+      std::make_unique<views::Combobox>(std::move(provider_model)));
+  panes_[pane_index].provider_selector->SetSelectedIndex(
+      coordinator_->GetProviderIndexForPane(pane_index));
+  panes_[pane_index].provider_selector->SetCallback(base::BindRepeating(
+      &ClashOfGptsView::OnProviderChanged, base::Unretained(this), pane_index));
+  panes_[pane_index].provider_selector->SetAccessibleName(
+      u"LLM Provider Selection for Pane " + base::NumberToString16(pane_index + 1));
+
+  // Add spacer
+  auto* spacer = header->AddChildView(std::make_unique<views::View>());
+  static_cast<views::BoxLayout*>(header->GetLayoutManager())
+      ->SetFlexForView(spacer, 1);
+
+  // Add open in new tab button
+  auto* open_button = header->AddChildView(
+      std::make_unique<views::ImageButton>(base::BindRepeating(
+          &ClashOfGptsView::OnOpenInNewTab, base::Unretained(this), pane_index)));
+  open_button->SetImageModel(
+      views::Button::STATE_NORMAL,
+      ui::ImageModel::FromVectorIcon(vector_icons::kLaunchIcon, ui::kColorIcon, 20));  // Increased icon size
+  open_button->SetAccessibleName(u"Open in new tab");
+  open_button->SetTooltipText(u"Open in new tab");
+  open_button->SetPreferredSize(gfx::Size(32, 32));  // Set button size
+  open_button->SetImageHorizontalAlignment(views::ImageButton::ALIGN_CENTER);
+  open_button->SetImageVerticalAlignment(views::ImageButton::ALIGN_MIDDLE);
+
+  // Create WebView
+  panes_[pane_index].web_view = pane_container->AddChildView(
+      std::make_unique<views::WebView>(coordinator_->GetBrowser().profile()));
+  panes_[pane_index].web_view->SetProperty(
+      views::kFlexBehaviorKey,
+      views::FlexSpecification(views::MinimumFlexSizeRule::kScaleToZero,
+                               views::MaximumFlexSizeRule::kUnbounded));
+
+  // Get WebContents from coordinator (it owns them)
+  content::WebContents* web_contents = coordinator_->GetOrCreateWebContentsForPane(pane_index);
+  if (web_contents) {
+    // Navigate to initial provider URL
+    size_t provider_index = coordinator_->GetProviderIndexForPane(pane_index);
+    const auto& providers = coordinator_->GetProviders();
+    if (provider_index < providers.size()) {
+      GURL provider_url = providers[provider_index].url;
+      web_contents->GetController().LoadURL(
+          provider_url,
+          content::Referrer(),
+          ui::PAGE_TRANSITION_AUTO_TOPLEVEL,
+          std::string());
+    }
+
+    // Set the WebContents in the WebView (WebView does NOT take ownership)
+    panes_[pane_index].web_view->SetWebContents(web_contents);
+    panes_[pane_index].web_view->SetVisible(true);
+  }
+
+  // Enable focus and accelerators
+  panes_[pane_index].web_view->SetFocusBehavior(views::View::FocusBehavior::ALWAYS);
+  panes_[pane_index].web_view->set_allow_accelerators(true);
+
+  return pane_container;
+}
+
+void ClashOfGptsView::OnProviderChanged(int pane_index) {
+  if (!panes_[pane_index].provider_selector) {
+    return;
+  }
+
+  auto selected_index = panes_[pane_index].provider_selector->GetSelectedIndex();
+  if (!selected_index || selected_index.value() >= coordinator_->GetProviders().size()) {
+    return;
+  }
+
+  coordinator_->SetProviderForPane(pane_index, selected_index.value());
+}
+
+void ClashOfGptsView::OnOpenInNewTab(int pane_index) {
+  content::WebContents* web_contents = GetWebContentsForPane(pane_index);
+  if (!web_contents) {
+    return;
+  }
+
+  GURL current_url = web_contents->GetURL();
+  if (!current_url.is_valid()) {
+    return;
+  }
+
+  // Open the current URL in a new tab
+  NavigateParams params(&coordinator_->GetBrowser(), current_url, 
+                       ui::PAGE_TRANSITION_LINK);
+  params.disposition = WindowOpenDisposition::NEW_FOREGROUND_TAB;
+  Navigate(&params);
+}
+
+void ClashOfGptsView::OnCopyContent() {
+  coordinator_->CopyContentToAll();
+}
+
+void ClashOfGptsView::HideFeedbackLabel() {
+  if (copy_feedback_label_ && copy_feedback_label_->GetWidget()) {
+    copy_feedback_label_->SetVisible(false);
+  }
+}
+
+void ClashOfGptsView::UpdatePaneCount(int new_count) {
+  if (new_count == static_cast<int>(panes_.size())) {
+    return;
+  }
+
+  // Update radio button state
+  one_pane_radio_->SetChecked(new_count == 1);
+  two_panes_radio_->SetChecked(new_count == 2);
+  three_panes_radio_->SetChecked(new_count == 3);
+
+  // Recreate panes with new count
+  RecreatePanesContainer();
+}
+
+void ClashOfGptsView::OnPaneCountChanged(int pane_count) {
+  coordinator_->SetPaneCount(pane_count);
+}
+
+void ClashOfGptsView::RecreatePanesContainer() {
+  // Clear existing panes
+  panes_container_->RemoveAllChildViews();
+  panes_.clear();
+  panes_.resize(coordinator_->GetPaneCount());
+
+  // Set up layout
+  auto* panes_layout = panes_container_->SetLayoutManager(
+      std::make_unique<views::FlexLayout>());
+  panes_layout->SetOrientation(views::LayoutOrientation::kHorizontal)
+      .SetMainAxisAlignment(views::LayoutAlignment::kStart)
+      .SetCrossAxisAlignment(views::LayoutAlignment::kStretch);
+
+  // Create panes
+  for (int i = 0; i < coordinator_->GetPaneCount(); ++i) {
+    auto pane_view = CreatePaneView(i);
+    pane_view->SetProperty(
+        views::kFlexBehaviorKey,
+        views::FlexSpecification(views::MinimumFlexSizeRule::kScaleToZero,
+                                 views::MaximumFlexSizeRule::kUnbounded)
+            .WithWeight(1));  // Equal weight for all panes
+    panes_container_->AddChildView(std::move(pane_view));
+
+    // Add separator between panes (except after the last one)
+    if (i < coordinator_->GetPaneCount() - 1) {
+      auto* separator = panes_container_->AddChildView(
+          std::make_unique<views::Separator>());
+      separator->SetOrientation(views::Separator::Orientation::kVertical);
+    }
+  }
+
+  // Force layout update
+  panes_container_->InvalidateLayout();
+  if (GetWidget()) {
+    GetWidget()->LayoutRootViewIfNecessary();
+  }
+}
+
+BEGIN_METADATA(ClashOfGptsView)
+END_METADATA
