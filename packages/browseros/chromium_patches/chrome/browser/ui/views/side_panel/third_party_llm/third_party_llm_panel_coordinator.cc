diff --git a/chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.cc b/chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.cc
new file mode 100644
index 0000000000000..f87c563fdd645
--- /dev/null
+++ b/chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.cc
@@ -0,0 +1,1163 @@
+// Copyright 2024 The Chromium Authors
+// Use of this source code is governed by a BSD-style license that can be
+// found in the LICENSE file.
+
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_panel_coordinator.h"
+
+#include <memory>
+#include <vector>
+
+#include "base/check.h"
+#include "base/check_deref.h"
+#include "build/build_config.h"
+#include "base/functional/callback.h"
+#include "ui/views/controls/menu/menu_runner.h"
+#include "ui/base/mojom/menu_source_type.mojom.h"
+#include "chrome/browser/ui/views/side_panel/third_party_llm/third_party_llm_view.h"
+#include "base/strings/utf_string_conversions.h"
+#include "chrome/browser/profiles/profile.h"
+#include "chrome/browser/ui/browser.h"
+#include "chrome/browser/ui/browser_list.h"
+#include "chrome/browser/ui/browser_window.h"
+#include "chrome/browser/ui/browser_window/public/browser_window_features.h"
+#include "chrome/browser/ui/views/chrome_layout_provider.h"
+#include "chrome/browser/ui/views/side_panel/side_panel_entry.h"
+#include "chrome/browser/ui/views/side_panel/side_panel_entry_id.h"
+#include "chrome/browser/ui/views/side_panel/side_panel_registry.h"
+#include "chrome/browser/ui/views/side_panel/side_panel_ui.h"
+#include "chrome/grit/generated_resources.h"
+#include "content/public/browser/web_contents.h"
+#include "ui/base/l10n/l10n_util.h"
+#include "ui/base/models/combobox_model.h"
+#include "ui/views/controls/button/image_button.h"
+#include "ui/views/controls/button/label_button.h"
+#include "ui/views/controls/combobox/combobox.h"
+#include "ui/views/controls/label.h"
+#include "ui/views/controls/separator.h"
+#include "ui/views/controls/webview/webview.h"
+#include "ui/views/layout/box_layout.h"
+#include "ui/views/layout/flex_layout.h"
+#include "ui/views/vector_icons.h"
+#include "components/vector_icons/vector_icons.h"
+#include "chrome/app/vector_icons/vector_icons.h"
+#include "components/prefs/pref_service.h"
+#include "components/user_prefs/user_prefs.h"
+#include "components/pref_registry/pref_registry_syncable.h"
+#include "chrome/browser/ui/browser_navigator.h"
+#include "chrome/browser/ui/browser_navigator_params.h"
+#include "chrome/browser/ui/tabs/tab_strip_model.h"
+#include "content/public/browser/browser_accessibility_state.h"
+#include "ui/accessibility/ax_node.h"
+#include "ui/accessibility/ax_node_data.h"
+#include "ui/accessibility/ax_enums.mojom.h"
+#include "ui/accessibility/ax_tree_update.h"
+#include "ui/base/clipboard/clipboard.h"
+#include "ui/base/clipboard/scoped_clipboard_writer.h"
+#include "chrome/browser/ui/browser_commands.h"
+#include "chrome/browser/ui/chrome_pages.h"
+#include "chrome/app/chrome_command_ids.h"
+#include "chrome/browser/ui/browser_tabstrip.h"
+#include "base/timer/timer.h"
+#include "base/task/sequenced_task_runner.h"
+#include "components/input/native_web_keyboard_event.h"
+#include "content/public/browser/render_widget_host_view.h"
+#include "third_party/skia/include/core/SkBitmap.h"
+#include "ui/gfx/codec/png_codec.h"
+#include "ui/gfx/image/image.h"
+#include "chrome/browser/file_select_helper.h"
+#include "chrome/browser/media/webrtc/media_capture_devices_dispatcher.h"
+#include "content/public/browser/file_select_listener.h"
+#include "third_party/blink/public/common/mediastream/media_stream_request.h"
+#include "content/public/browser/render_frame_host.h"
+#include "chrome/browser/browseros/metrics/browseros_metrics.h"
+#include "chrome/browser/ui/views/side_panel/clash_of_gpts/clash_of_gpts_coordinator.h"
+
+namespace {
+
+// Preference names
+const char kThirdPartyLlmProvidersPref[] = "browseros.third_party_llm.providers";
+const char kThirdPartyLlmSelectedProviderPref[] = "browseros.third_party_llm.selected_provider";
+
+// ComboboxModel for LLM provider selection
+class LlmProviderComboboxModel : public ui::ComboboxModel {
+ public:
+  explicit LlmProviderComboboxModel(const std::vector<LlmProviderInfo>* providers)
+      : providers_(providers) {}
+  ~LlmProviderComboboxModel() override = default;
+
+  // ui::ComboboxModel:
+  size_t GetItemCount() const override {
+    return providers_ ? providers_->size() : 0;
+  }
+
+  std::u16string GetItemAt(size_t index) const override {
+    if (!providers_ || index >= providers_->size()) {
+      NOTREACHED();
+    }
+    return (*providers_)[index].name;
+  }
+
+ private:
+  raw_ptr<const std::vector<LlmProviderInfo>> providers_;
+};
+
+}  // namespace
+
+ThirdPartyLlmPanelCoordinator::ThirdPartyLlmPanelCoordinator(
+    Profile* profile,
+    TabStripModel* tab_strip_model)
+    : profile_(CHECK_DEREF(profile)),
+      tab_strip_model_(CHECK_DEREF(tab_strip_model)),
+      feedback_timer_(std::make_unique<base::OneShotTimer>()) {
+  // Register for early cleanup notifications
+  browser_list_observation_.Observe(BrowserList::GetInstance());
+  profile_observation_.Observe(&profile_.get());
+
+  // Load providers from preferences
+  LoadProvidersFromPrefs();
+}
+
+ThirdPartyLlmPanelCoordinator::~ThirdPartyLlmPanelCoordinator() {
+  // Destructor should be minimal - cleanup already done in observer methods
+  // The ScopedObservation objects will automatically unregister
+}
+
+void ThirdPartyLlmPanelCoordinator::CreateAndRegisterEntry(
+    SidePanelRegistry* global_registry) {
+  auto entry = std::make_unique<SidePanelEntry>(
+      SidePanelEntry::Key(SidePanelEntry::Id::kThirdPartyLlm),
+      base::BindRepeating(
+          &ThirdPartyLlmPanelCoordinator::CreateThirdPartyLlmWebView,
+          base::Unretained(this)),
+      base::RepeatingCallback<int()>());
+  
+  global_registry->Register(std::move(entry));
+}
+
+std::unique_ptr<views::View>
+ThirdPartyLlmPanelCoordinator::CreateThirdPartyLlmWebView(
+    SidePanelEntryScope& scope) {
+  // Save current state before reloading preferences
+  size_t previous_size = providers_.size();
+  if (owned_web_contents_) {
+    GURL current_url = owned_web_contents_->GetURL();
+    if (current_url.is_valid()) {
+      last_urls_[current_provider_index_] = current_url;
+    }
+  }
+
+  // Reload providers from preferences
+  LoadProvidersFromPrefs();
+
+  // If provider list size changed, reset to first provider for safety
+  // This handles providers being added/removed while panel was closed
+  if (providers_.size() != previous_size) {
+    LOG(INFO) << "[browseros] Provider list size changed from " << previous_size
+              << " to " << providers_.size() << ", resetting to first provider";
+    current_provider_index_ = 0;
+    if (PrefService* prefs = GetProfile()->GetPrefs()) {
+      prefs->SetInteger(kThirdPartyLlmSelectedProviderPref, 0);
+    }
+  }
+
+  // Cancel any pending timer callbacks before resetting UI pointers
+  if (feedback_timer_ && feedback_timer_->IsRunning()) {
+    feedback_timer_->Stop();
+  }
+
+  // Reset UI pointers when creating new view
+  web_view_ = nullptr;
+  provider_selector_ = nullptr;
+  copy_feedback_label_ = nullptr;
+  menu_button_ = nullptr;
+
+  // Stop observing any previous views to prevent dangling observations.
+  view_observation_.RemoveAllObservations();
+
+  // Create the main container using our custom view that handles cleanup
+  auto container = std::make_unique<ThirdPartyLlmView>();
+  auto* container_layout = container->SetLayoutManager(std::make_unique<views::FlexLayout>());
+  container_layout->SetOrientation(views::LayoutOrientation::kVertical)
+      .SetMainAxisAlignment(views::LayoutAlignment::kStart)
+      .SetCrossAxisAlignment(views::LayoutAlignment::kStretch);
+  
+  // Create header container with vertical layout for dropdown and feedback
+  auto* header_container = container->AddChildView(std::make_unique<views::View>());
+  header_container->SetLayoutManager(std::make_unique<views::BoxLayout>(
+      views::BoxLayout::Orientation::kVertical, 
+      gfx::Insets::TLBR(8, 12, 4, 12), 
+      4));
+  
+  // Create header row with dropdown and buttons
+  auto* header = header_container->AddChildView(std::make_unique<views::View>());
+  header->SetLayoutManager(std::make_unique<views::BoxLayout>(
+      views::BoxLayout::Orientation::kHorizontal, 
+      gfx::Insets(), 
+      12));  // Increased spacing between elements
+  
+  // Add dropdown
+  auto provider_model = std::make_unique<LlmProviderComboboxModel>(&providers_);
+  provider_selector_ = header->AddChildView(
+      std::make_unique<views::Combobox>(std::move(provider_model)));
+  provider_selector_->SetSelectedIndex(current_provider_index_);
+  provider_selector_->SetCallback(base::BindRepeating(
+      &ThirdPartyLlmPanelCoordinator::OnProviderChanged,
+      weak_factory_.GetWeakPtr()));
+  provider_selector_->SetAccessibleName(u"LLM Provider Selection");
+
+  // Add feedback label below dropdown (initially hidden)
+  copy_feedback_label_ = header_container->AddChildView(
+      std::make_unique<views::Label>(u""));
+  copy_feedback_label_->SetVisible(false);
+  copy_feedback_label_->SetEnabledColor(ui::kColorLabelForegroundSecondary);
+  copy_feedback_label_->SetFontList(
+      copy_feedback_label_->font_list().DeriveWithSizeDelta(-1));
+  
+  // Observe UI elements so we can reset pointers when they are destroyed.
+  view_observation_.AddObservation(copy_feedback_label_);
+  view_observation_.AddObservation(provider_selector_);
+  
+  // Add flexible spacer
+  views::BoxLayout* box_layout = static_cast<views::BoxLayout*>(header->GetLayoutManager());
+  box_layout->SetFlexForView(header->AddChildView(std::make_unique<views::View>()), 1);
+  
+  // Add copy content button
+  auto* copy_button = header->AddChildView(
+      std::make_unique<views::ImageButton>(base::BindRepeating(
+          &ThirdPartyLlmPanelCoordinator::OnCopyContent,
+          weak_factory_.GetWeakPtr())));
+  copy_button->SetImageModel(
+      views::Button::STATE_NORMAL,
+      ui::ImageModel::FromVectorIcon(vector_icons::kContentCopyIcon, ui::kColorIcon, 20));
+  copy_button->SetAccessibleName(u"Copy page content");
+  copy_button->SetTooltipText(u"Copy main page content to clipboard");
+  copy_button->SetPreferredSize(gfx::Size(32, 32));
+  copy_button->SetImageHorizontalAlignment(views::ImageButton::ALIGN_CENTER);
+  copy_button->SetImageVerticalAlignment(views::ImageButton::ALIGN_MIDDLE);
+  
+  // Add screenshot button
+  auto* screenshot_button = header->AddChildView(
+      std::make_unique<views::ImageButton>(base::BindRepeating(
+          &ThirdPartyLlmPanelCoordinator::OnScreenshotContent,
+          weak_factory_.GetWeakPtr())));
+  screenshot_button->SetImageModel(
+      views::Button::STATE_NORMAL,
+      ui::ImageModel::FromVectorIcon(vector_icons::kPhotoChromeRefreshIcon, ui::kColorIcon, 20));
+  screenshot_button->SetAccessibleName(u"Take screenshot");
+  screenshot_button->SetTooltipText(u"Capture visible page screenshot to clipboard");
+  screenshot_button->SetPreferredSize(gfx::Size(32, 32));
+  screenshot_button->SetImageHorizontalAlignment(views::ImageButton::ALIGN_CENTER);
+  screenshot_button->SetImageVerticalAlignment(views::ImageButton::ALIGN_MIDDLE);
+
+  // Add refresh button
+  auto* refresh_button = header->AddChildView(
+      std::make_unique<views::ImageButton>(base::BindRepeating(
+          &ThirdPartyLlmPanelCoordinator::OnRefreshContent,
+          weak_factory_.GetWeakPtr())));
+  refresh_button->SetImageModel(
+      views::Button::STATE_NORMAL,
+      ui::ImageModel::FromVectorIcon(vector_icons::kReloadIcon, ui::kColorIcon, 20));
+  refresh_button->SetAccessibleName(u"Refresh");
+  refresh_button->SetTooltipText(u"Reload default page for current provider");
+  refresh_button->SetPreferredSize(gfx::Size(32, 32));
+  refresh_button->SetImageHorizontalAlignment(views::ImageButton::ALIGN_CENTER);
+  refresh_button->SetImageVerticalAlignment(views::ImageButton::ALIGN_MIDDLE);
+
+  // Add open in new tab button
+  auto* open_button = header->AddChildView(
+      std::make_unique<views::ImageButton>(base::BindRepeating(
+          &ThirdPartyLlmPanelCoordinator::OnOpenInNewTab,
+          weak_factory_.GetWeakPtr())));
+  open_button->SetImageModel(
+      views::Button::STATE_NORMAL,
+      ui::ImageModel::FromVectorIcon(vector_icons::kLaunchIcon, ui::kColorIcon, 20));
+  open_button->SetAccessibleName(u"Open in new tab");
+  open_button->SetTooltipText(u"Open in new tab");
+  open_button->SetPreferredSize(gfx::Size(32, 32));
+  open_button->SetImageHorizontalAlignment(views::ImageButton::ALIGN_CENTER);
+  open_button->SetImageVerticalAlignment(views::ImageButton::ALIGN_MIDDLE);
+  
+  // Add options menu button (3-dot menu)
+  menu_button_ = header->AddChildView(
+      std::make_unique<views::ImageButton>(base::BindRepeating(
+          &ThirdPartyLlmPanelCoordinator::ShowOptionsMenu,
+          weak_factory_.GetWeakPtr())));
+  menu_button_->SetImageModel(
+      views::Button::STATE_NORMAL,
+      ui::ImageModel::FromVectorIcon(kBrowserToolsIcon, ui::kColorIcon, 20));
+  menu_button_->SetAccessibleName(u"More options");
+  menu_button_->SetTooltipText(u"More options");
+  menu_button_->SetPreferredSize(gfx::Size(32, 32));
+  menu_button_->SetImageHorizontalAlignment(views::ImageButton::ALIGN_CENTER);
+  menu_button_->SetImageVerticalAlignment(views::ImageButton::ALIGN_MIDDLE);
+  
+  // Observe the menu button
+  view_observation_.AddObservation(menu_button_);
+  
+  // Add separator
+  container->AddChildView(std::make_unique<views::Separator>());
+  
+  // Create WebView
+  web_view_ = container->AddChildView(
+      std::make_unique<views::WebView>(GetProfile()));
+  web_view_->SetProperty(
+      views::kFlexBehaviorKey,
+      views::FlexSpecification(views::MinimumFlexSizeRule::kScaleToZero,
+                               views::MaximumFlexSizeRule::kUnbounded));
+  
+  // Observe UI elements so we can reset pointers when they are destroyed.
+  view_observation_.AddObservation(web_view_);
+  
+  // Create WebContents if we don't have one yet
+  if (!owned_web_contents_) {
+    content::WebContents::CreateParams params(GetProfile());
+    owned_web_contents_ = content::WebContents::Create(params);
+
+    // Set this as the delegate to handle keyboard events
+    owned_web_contents_->SetDelegate(this);
+  }
+
+  // Navigate to initial provider (use last URL if available)
+  GURL provider_url;
+  if (current_provider_index_ < providers_.size()) {
+    auto it = last_urls_.find(current_provider_index_);
+    if (it != last_urls_.end() && it->second.is_valid()) {
+      provider_url = it->second;
+    } else {
+      provider_url = providers_[current_provider_index_].url;
+    }
+  }
+
+  if (provider_url.is_valid()) {
+    owned_web_contents_->GetController().LoadURL(
+        provider_url,
+        content::Referrer(),
+        ui::PAGE_TRANSITION_AUTO_TOPLEVEL,
+        std::string());
+  }
+
+  // Set the WebContents in the WebView (WebView does NOT take ownership)
+  // We pass the raw pointer but retain ownership via owned_web_contents_
+  web_view_->SetWebContents(owned_web_contents_.get());
+  web_view_->SetVisible(true);
+  
+  // Tell our custom container about the WebView for proper cleanup
+  container->SetWebView(web_view_);
+
+  // Observe the WebContents
+  Observe(owned_web_contents_.get());
+
+  // Enable focus for the WebView to handle keyboard events properly
+  web_view_->SetFocusBehavior(views::View::FocusBehavior::ALWAYS);
+
+  // Allow accelerators (keyboard shortcuts) to be processed
+  web_view_->set_allow_accelerators(true);
+
+  // Add separator before footer
+  container->AddChildView(std::make_unique<views::Separator>());
+
+  // Create footer with keyboard shortcuts
+  auto* footer = container->AddChildView(std::make_unique<views::View>());
+  footer->SetLayoutManager(std::make_unique<views::BoxLayout>(
+      views::BoxLayout::Orientation::kHorizontal,
+      gfx::Insets::TLBR(6, 12, 6, 12),
+      8));
+  
+  // Add keyboard icon
+  auto* keyboard_icon = footer->AddChildView(std::make_unique<views::Label>(u"⌨️"));
+  keyboard_icon->SetEnabledColor(ui::kColorLabelForegroundSecondary);
+  
+  // Add shortcut text
+#if BUILDFLAG(IS_MAC)
+  auto* shortcuts_label = footer->AddChildView(
+      std::make_unique<views::Label>(u"Toggle: Option+K  •  Next: Option+L"));
+#else
+  auto* shortcuts_label = footer->AddChildView(
+      std::make_unique<views::Label>(u"Toggle: Alt+K  •  Next: Alt+L"));
+#endif
+  shortcuts_label->SetEnabledColor(ui::kColorLabelForegroundSecondary);
+  shortcuts_label->SetFontList(
+      shortcuts_label->font_list().DeriveWithSizeDelta(-1));
+  
+  browseros_metrics::BrowserOSMetrics::Log("llmchat.created");
+  
+  return container;
+}
+
+void ThirdPartyLlmPanelCoordinator::OnProviderChanged() {
+  if (!provider_selector_)
+    return;
+
+  auto selected_index = provider_selector_->GetSelectedIndex();
+  if (!selected_index || selected_index.value() >= providers_.size())
+    return;
+
+  DoProviderChange(selected_index.value());
+}
+
+std::vector<LlmProviderInfo> ThirdPartyLlmPanelCoordinator::GetDefaultProviders() const {
+  std::vector<LlmProviderInfo> defaults;
+  defaults.push_back({u"ChatGPT", GURL("https://chatgpt.com")});
+  defaults.push_back({u"Claude", GURL("https://claude.ai")});
+  defaults.push_back({u"Grok", GURL("https://grok.com")});
+  defaults.push_back({u"Gemini", GURL("https://gemini.google.com")});
+  defaults.push_back({u"Perplexity", GURL("https://www.perplexity.ai")});
+  return defaults;
+}
+
+void ThirdPartyLlmPanelCoordinator::LoadProvidersFromPrefs() {
+  PrefService* prefs = GetProfile()->GetPrefs();
+  if (!prefs) {
+    LOG(ERROR) << "[browseros] Failed to get PrefService";
+    providers_ = GetDefaultProviders();
+    return;
+  }
+
+  const base::Value::List& providers_list = prefs->GetList(kThirdPartyLlmProvidersPref);
+
+  providers_.clear();
+
+  if (!providers_list.empty()) {
+    for (const base::Value& item : providers_list) {
+      if (!item.is_dict()) {
+        LOG(WARNING) << "[browseros] Invalid provider entry (not a dict), skipping";
+        continue;
+      }
+
+      const std::string* name = item.GetDict().FindString("name");
+      const std::string* url = item.GetDict().FindString("url");
+
+      if (!name || name->empty()) {
+        LOG(WARNING) << "[browseros] Provider missing name, skipping";
+        continue;
+      }
+
+      if (!url || url->empty()) {
+        LOG(WARNING) << "[browseros] Provider missing URL, skipping";
+        continue;
+      }
+
+      GURL provider_url(*url);
+      if (!provider_url.is_valid()) {
+        LOG(WARNING) << "[browseros] Invalid provider URL: " << *url;
+        continue;
+      }
+
+      providers_.push_back({base::UTF8ToUTF16(*name), provider_url});
+    }
+  }
+
+  // If no valid providers loaded, use defaults
+  if (providers_.empty()) {
+    LOG(INFO) << "[browseros] No providers in prefs, using defaults";
+    providers_ = GetDefaultProviders();
+    SaveProvidersToPrefs();
+  }
+
+  // Load selected provider index
+  int selected_index = prefs->GetInteger(kThirdPartyLlmSelectedProviderPref);
+  if (selected_index < 0 || static_cast<size_t>(selected_index) >= providers_.size()) {
+    LOG(WARNING) << "[browseros] Invalid selected provider index: " << selected_index
+                 << ", resetting to 0";
+    current_provider_index_ = 0;
+    prefs->SetInteger(kThirdPartyLlmSelectedProviderPref, 0);
+  } else {
+    current_provider_index_ = static_cast<size_t>(selected_index);
+  }
+}
+
+void ThirdPartyLlmPanelCoordinator::SaveProvidersToPrefs() {
+  PrefService* prefs = GetProfile()->GetPrefs();
+  if (!prefs) {
+    LOG(ERROR) << "[browseros] Failed to get PrefService for saving";
+    return;
+  }
+
+  base::Value::List providers_list;
+
+  for (const auto& provider : providers_) {
+    base::Value::Dict provider_dict;
+    provider_dict.Set("name", base::UTF16ToUTF8(provider.name));
+    provider_dict.Set("url", provider.url.spec());
+    providers_list.Append(std::move(provider_dict));
+  }
+
+  prefs->SetList(kThirdPartyLlmProvidersPref, std::move(providers_list));
+}
+
+void ThirdPartyLlmPanelCoordinator::DoProviderChange(size_t new_provider_index) {
+  // Prevent re-entrancy and overlapping updates.
+  if (provider_change_in_progress_ || new_provider_index == current_provider_index_)
+    return;
+
+  if (new_provider_index >= providers_.size()) {
+    LOG(ERROR) << "[browseros] Invalid provider index: " << new_provider_index;
+    return;
+  }
+
+  provider_change_in_progress_ = true;
+
+  browseros_metrics::BrowserOSMetrics::Log("llmchat.provider.changed");
+
+  if (owned_web_contents_) {
+    GURL current_url = owned_web_contents_->GetURL();
+    if (current_url.is_valid()) {
+      last_urls_[current_provider_index_] = current_url;
+    }
+  }
+
+  current_provider_index_ = new_provider_index;
+
+  // Persist preference.
+  if (PrefService* prefs = GetProfile()->GetPrefs()) {
+    prefs->SetInteger(kThirdPartyLlmSelectedProviderPref, static_cast<int>(current_provider_index_));
+  }
+
+  // Determine URL to load.
+  GURL provider_url;
+  auto it = last_urls_.find(current_provider_index_);
+  provider_url = (it != last_urls_.end() && it->second.is_valid())
+                     ? it->second
+                     : providers_[current_provider_index_].url;
+
+  if (owned_web_contents_) {
+    owned_web_contents_->GetController().LoadURL(
+        provider_url, content::Referrer(), ui::PAGE_TRANSITION_AUTO_TOPLEVEL, std::string());
+  }
+
+  provider_change_in_progress_ = false;
+}
+
+void ThirdPartyLlmPanelCoordinator::OnRefreshContent() {
+  if (!owned_web_contents_ || current_provider_index_ >= providers_.size()) {
+    return;
+  }
+
+  // Get the default URL for the current provider
+  GURL provider_url = providers_[current_provider_index_].url;
+
+  // Navigate to the default URL
+  owned_web_contents_->GetController().LoadURL(
+      provider_url,
+      content::Referrer(),
+      ui::PAGE_TRANSITION_AUTO_TOPLEVEL,
+      std::string());
+
+  // Clear the saved URL for this provider so it uses the default next time
+  last_urls_.erase(current_provider_index_);
+}
+
+void ThirdPartyLlmPanelCoordinator::OnOpenInNewTab() {
+  if (!owned_web_contents_) {
+    return;
+  }
+
+  GURL current_url = owned_web_contents_->GetURL();
+  if (!current_url.is_valid()) {
+    return;
+  }
+
+  // Create new WebContents and navigate to the URL
+  content::WebContents::CreateParams params(GetProfile());
+  std::unique_ptr<content::WebContents> new_contents =
+      content::WebContents::Create(params);
+  new_contents->GetController().LoadURL(
+      current_url, content::Referrer(), ui::PAGE_TRANSITION_LINK, std::string());
+
+  // Add to tab strip
+  GetTabStripModel()->AddWebContents(
+      std::move(new_contents), -1,
+      ui::PAGE_TRANSITION_LINK, AddTabTypes::ADD_ACTIVE);
+}
+
+void ThirdPartyLlmPanelCoordinator::OnCopyContent() {
+  // Get the active tab's web contents
+  TabStripModel* tab_strip_model = GetTabStripModel();
+  if (!tab_strip_model) {
+    return;
+  }
+
+  content::WebContents* active_contents = tab_strip_model->GetActiveWebContents();
+  if (!active_contents) {
+    return;
+  }
+  
+  // Store the title and URL for later use
+  page_title_ = active_contents->GetTitle();
+  page_url_ = active_contents->GetVisibleURL();
+  
+  // Request accessibility tree snapshot
+  active_contents->RequestAXTreeSnapshot(
+      base::BindOnce(&ThirdPartyLlmPanelCoordinator::OnAccessibilityTreeReceived,
+                     weak_factory_.GetWeakPtr()),
+      ui::AXMode::kWebContents,  // Request web contents mode
+      0,  // max_nodes (0 = no limit)
+      base::Seconds(5),  // timeout
+      content::WebContents::AXTreeSnapshotPolicy::kSameOriginDirectDescendants);
+}
+
+void ThirdPartyLlmPanelCoordinator::OnScreenshotContent() {
+  // Get the active tab's web contents
+  TabStripModel* tab_strip_model = GetTabStripModel();
+  if (!tab_strip_model) {
+    return;
+  }
+
+  content::WebContents* active_contents = tab_strip_model->GetActiveWebContents();
+  if (!active_contents) {
+    return;
+  }
+
+  // Get the render widget host
+  content::RenderWidgetHostView* view = active_contents->GetRenderWidgetHostView();
+  if (!view) {
+    return;
+  }
+
+  // For now, just capture the visible viewport
+  // Full page screenshot would require DevTools protocol or RenderFrameHostImpl access
+  view->CopyFromSurface(
+      gfx::Rect(),  // Empty rect = full visible surface
+      gfx::Size(),  // Empty size = original size
+      base::BindOnce([](base::WeakPtr<ThirdPartyLlmPanelCoordinator> coordinator,
+                        const SkBitmap& bitmap) {
+        if (!coordinator) {
+          return;
+        }
+        gfx::Image image;
+        if (!bitmap.drawsNothing()) {
+          image = gfx::Image::CreateFrom1xBitmap(bitmap);
+        }
+        coordinator->OnScreenshotCaptured(image);
+      }, weak_factory_.GetWeakPtr()));
+}
+
+
+void ThirdPartyLlmPanelCoordinator::OnScreenshotCaptured(
+    const gfx::Image& image) {
+  if (image.IsEmpty()) {
+    if (copy_feedback_label_) {
+      copy_feedback_label_->SetText(u"Failed to capture screenshot");
+      copy_feedback_label_->SetVisible(true);
+
+      // Start timer to hide message
+      if (feedback_timer_->IsRunning()) {
+        feedback_timer_->Stop();
+      }
+      feedback_timer_->Start(FROM_HERE, base::Seconds(2.5),
+          base::BindOnce(&ThirdPartyLlmPanelCoordinator::HideFeedbackLabel,
+                         weak_factory_.GetWeakPtr()));
+    }
+    return;
+  }
+
+  // Copy image to clipboard
+  ui::ScopedClipboardWriter clipboard_writer(ui::ClipboardBuffer::kCopyPaste);
+  clipboard_writer.WriteImage(image.AsBitmap());
+  
+  browseros_metrics::BrowserOSMetrics::Log("llmchat.screenshot.captured");
+
+  // Show success feedback
+  if (copy_feedback_label_) {
+    copy_feedback_label_->SetText(u"Screenshot copied to clipboard");
+    copy_feedback_label_->SetVisible(true);
+
+    // Start timer to hide message
+    if (feedback_timer_->IsRunning()) {
+      feedback_timer_->Stop();
+    }
+    feedback_timer_->Start(FROM_HERE, base::Seconds(2.5),
+        base::BindOnce(&ThirdPartyLlmPanelCoordinator::HideFeedbackLabel,
+                       weak_factory_.GetWeakPtr()));
+  }
+}
+
+void ThirdPartyLlmPanelCoordinator::OnAccessibilityTreeReceived(
+    ui::AXTreeUpdate& update) {
+  // Build a map of node IDs to node data for easy lookup
+  std::map<ui::AXNodeID, const ui::AXNodeData*> node_map;
+  for (const auto& node_data : update.nodes) {
+    node_map[node_data.id] = &node_data;
+  }
+  
+  // Find the root node
+  ui::AXNodeID root_id = update.root_id;
+  if (node_map.find(root_id) == node_map.end()) {
+    LOG(ERROR) << "Root node not found in tree update";
+    return;
+  }
+  
+  // Extract text from the accessibility tree recursively
+  std::u16string extracted_text;
+  ExtractTextFromNodeData(node_map[root_id], node_map, &extracted_text);
+  
+  // Clean up text - remove excessive whitespace
+  if (!extracted_text.empty()) {
+    // Simple cleanup of multiple spaces
+    size_t pos = 0;
+    while ((pos = extracted_text.find(u"  ", pos)) != std::u16string::npos) {
+      extracted_text.replace(pos, 2, u" ");
+    }
+    
+    // Format the final output
+    std::u16string formatted_output = u"----------- WEB PAGE -----------\n\n";
+    formatted_output += u"TITLE: " + page_title_ + u"\n\n";
+    formatted_output += u"URL: " + base::UTF8ToUTF16(page_url_.spec()) + u"\n\n";
+    formatted_output += u"CONTENT:\n\n" + extracted_text;
+    formatted_output += u" ------------------------------------\n\n";
+    formatted_output += u"USER PROMPT:\n\n";
+    
+    // Copy to clipboard
+    ui::ScopedClipboardWriter clipboard_writer(ui::ClipboardBuffer::kCopyPaste);
+    clipboard_writer.WriteText(formatted_output);
+    
+    browseros_metrics::BrowserOSMetrics::Log("llmchat.content.copied");
+    
+    // Show feedback message
+    if (copy_feedback_label_) {
+      copy_feedback_label_->SetText(u"Content copied to clipboard");
+      copy_feedback_label_->SetVisible(true);
+
+      // Cancel any existing timer
+      if (feedback_timer_->IsRunning()) {
+        feedback_timer_->Stop();
+      }
+
+      // Start timer to hide message after 2.5 seconds
+      feedback_timer_->Start(FROM_HERE, base::Seconds(2.5),
+          base::BindOnce(&ThirdPartyLlmPanelCoordinator::HideFeedbackLabel,
+                         weak_factory_.GetWeakPtr()));
+    }
+  }
+}
+
+void ThirdPartyLlmPanelCoordinator::HideFeedbackLabel() {
+  // The timer may fire after the UI element has been destroyed (e.g. the side
+  // panel was closed). Guard against use-after-free by checking that the raw
+  // pointer is still valid (we clear it in OnViewIsDeleting).
+  if (!copy_feedback_label_)
+    return;
+
+  if (copy_feedback_label_->GetWidget()) {
+    copy_feedback_label_->SetVisible(false);
+  }
+}
+
+void ThirdPartyLlmPanelCoordinator::OnViewIsDeleting(views::View* observed_view) {
+  // Stop any pending timer that could reference destroyed elements.
+  if (observed_view == copy_feedback_label_) {
+    if (feedback_timer_ && feedback_timer_->IsRunning()) {
+      feedback_timer_->Stop();
+    }
+    copy_feedback_label_ = nullptr;
+  }
+
+  if (observed_view == provider_selector_) {
+    provider_selector_ = nullptr;
+  }
+
+  if (observed_view == web_view_) {
+    // Just clear our pointer. DO NOT call methods on the view being destroyed!
+    web_view_ = nullptr;
+  }
+
+  if (observed_view == menu_button_) {
+    menu_button_ = nullptr;
+  }
+
+  // Remove observation for this view.
+  view_observation_.RemoveObservation(observed_view);
+}
+
+
+void ThirdPartyLlmPanelCoordinator::ExtractTextFromNodeData(
+    const ui::AXNodeData* node,
+    const std::map<ui::AXNodeID, const ui::AXNodeData*>& node_map,
+    std::u16string* output) {
+  if (!node || !output) {
+    return;
+  }
+  
+  // Skip UI elements and navigation
+  if (node->role == ax::mojom::Role::kButton ||
+      node->role == ax::mojom::Role::kNavigation ||
+      node->role == ax::mojom::Role::kBanner ||
+      node->role == ax::mojom::Role::kComplementary ||
+      node->role == ax::mojom::Role::kContentInfo ||
+      node->role == ax::mojom::Role::kForm ||
+      node->role == ax::mojom::Role::kSearch ||
+      node->role == ax::mojom::Role::kMenu ||
+      node->role == ax::mojom::Role::kMenuBar ||
+      node->role == ax::mojom::Role::kMenuItem ||
+      node->role == ax::mojom::Role::kToolbar) {
+    // For these elements, still traverse children but don't extract their text
+    for (ui::AXNodeID child_id : node->child_ids) {
+      auto it = node_map.find(child_id);
+      if (it != node_map.end()) {
+        ExtractTextFromNodeData(it->second, node_map, output);
+      }
+    }
+    return;
+  }
+  
+  // Check if this is a text-containing element
+  bool is_text_element = (node->role == ax::mojom::Role::kStaticText ||
+                         node->role == ax::mojom::Role::kInlineTextBox);
+  
+  // Extract text if this is a text element
+  if (is_text_element) {
+    std::u16string text;
+    if (node->HasStringAttribute(ax::mojom::StringAttribute::kName)) {
+      text = node->GetString16Attribute(ax::mojom::StringAttribute::kName);
+    }
+    
+    if (text.empty() && node->HasStringAttribute(ax::mojom::StringAttribute::kValue)) {
+      text = node->GetString16Attribute(ax::mojom::StringAttribute::kValue);
+    }
+    
+    if (!text.empty()) {
+      // Add appropriate spacing
+      if (!output->empty() && output->back() != ' ' && output->back() != '\n') {
+        *output += u" ";
+      }
+      *output += text;
+    }
+  }
+  
+  // Handle line breaks
+  if (node->role == ax::mojom::Role::kLineBreak) {
+    *output += u"\n";
+  }
+  
+  // Add paragraph breaks for block-level elements
+  bool needs_paragraph_break = (node->role == ax::mojom::Role::kParagraph ||
+                               node->role == ax::mojom::Role::kHeading ||
+                               node->role == ax::mojom::Role::kListItem ||
+                               node->role == ax::mojom::Role::kBlockquote ||
+                               node->role == ax::mojom::Role::kArticle ||
+                               node->role == ax::mojom::Role::kSection);
+  
+  if (needs_paragraph_break && !output->empty() && output->back() != '\n') {
+    *output += u"\n\n";
+  }
+  
+  // Recursively process children for all elements
+  for (ui::AXNodeID child_id : node->child_ids) {
+    auto it = node_map.find(child_id);
+    if (it != node_map.end()) {
+      ExtractTextFromNodeData(it->second, node_map, output);
+    }
+  }
+  
+  // Add paragraph break after block-level elements if they had content
+  if (needs_paragraph_break && !output->empty() && output->back() != '\n') {
+    *output += u"\n\n";
+  }
+}
+
+bool ThirdPartyLlmPanelCoordinator::HandleKeyboardEvent(
+    content::WebContents* source,
+    const input::NativeWebKeyboardEvent& event) {
+  // Get the focused view - should be our WebView
+  if (!web_view_ || !web_view_->GetWidget())
+    return false;
+    
+  // Use the unhandled keyboard event handler to process the event
+  return unhandled_keyboard_event_handler_.HandleKeyboardEvent(
+      event, web_view_->GetFocusManager());
+}
+
+content::WebContents* ThirdPartyLlmPanelCoordinator::AddNewContents(
+    content::WebContents* source,
+    std::unique_ptr<content::WebContents> new_contents,
+    const GURL& target_url,
+    WindowOpenDisposition disposition,
+    const blink::mojom::WindowFeatures& window_features,
+    bool user_gesture,
+    bool* was_blocked) {
+  // Only allow popups triggered by user gesture
+  if (!user_gesture) {
+    if (was_blocked) {
+      *was_blocked = true;
+    }
+    return nullptr;
+  }
+
+  // For popup windows and new tabs, add to the tab strip
+  if (disposition == WindowOpenDisposition::NEW_POPUP ||
+      disposition == WindowOpenDisposition::NEW_FOREGROUND_TAB ||
+      disposition == WindowOpenDisposition::NEW_BACKGROUND_TAB ||
+      disposition == WindowOpenDisposition::NEW_WINDOW) {
+    int add_types = (disposition == WindowOpenDisposition::NEW_FOREGROUND_TAB)
+                        ? AddTabTypes::ADD_ACTIVE
+                        : AddTabTypes::ADD_NONE;
+    GetTabStripModel()->AddWebContents(
+        std::move(new_contents), -1, ui::PAGE_TRANSITION_LINK, add_types);
+  }
+
+  return nullptr;
+}
+
+void ThirdPartyLlmPanelCoordinator::RunFileChooser(
+    content::RenderFrameHost* render_frame_host,
+    scoped_refptr<content::FileSelectListener> listener,
+    const blink::mojom::FileChooserParams& params) {
+  // Use FileSelectHelper to handle file selection, same as regular browser tabs
+  FileSelectHelper::RunFileChooser(render_frame_host, std::move(listener), params);
+}
+
+void ThirdPartyLlmPanelCoordinator::RequestMediaAccessPermission(
+    content::WebContents* web_contents,
+    const content::MediaStreamRequest& request,
+    content::MediaResponseCallback callback) {
+  LOG(INFO) << "[browseros] Media access requested for origin: "
+            << request.url_origin.Serialize()
+            << " audio: " << (request.audio_type !=
+                blink::mojom::MediaStreamType::NO_SERVICE)
+            << " video: " << (request.video_type !=
+                blink::mojom::MediaStreamType::NO_SERVICE);
+
+  // Check if this is one of our known LLM providers by comparing against all providers
+  const GURL origin_url = request.url_origin.GetURL();
+  bool is_known_llm = false;
+
+  // Check all LLM providers
+  for (const auto& provider : providers_) {
+    if (origin_url.host() == provider.url.host()) {
+      is_known_llm = true;
+      break;
+    }
+  }
+
+  if (is_known_llm) {
+    // Auto-grant permission for known LLM sites in the side panel
+    // This is safe because:
+    // 1. These are reputable sites
+    // 2. User explicitly opened them in the LLM panel
+    // 3. Voice input is expected functionality
+
+    LOG(INFO) << "[browseros] Auto-granting media permission for trusted LLM: "
+              << origin_url.host();
+
+    // Get available audio devices
+    const blink::MediaStreamDevices& audio_devices =
+        MediaCaptureDevicesDispatcher::GetInstance()->GetAudioCaptureDevices();
+
+    // Create response with the default audio device
+    blink::mojom::StreamDevicesSetPtr stream_devices_set =
+        blink::mojom::StreamDevicesSet::New();
+
+    if (request.audio_type != blink::mojom::MediaStreamType::NO_SERVICE &&
+        !audio_devices.empty()) {
+      blink::mojom::StreamDevicesPtr devices = blink::mojom::StreamDevices::New();
+      devices->audio_device = audio_devices.front();
+      stream_devices_set->stream_devices.push_back(std::move(devices));
+    }
+
+    // Return success with granted devices
+    std::move(callback).Run(
+        *stream_devices_set,
+        blink::mojom::MediaStreamRequestResult::OK,
+        std::unique_ptr<content::MediaStreamUI>());
+  } else {
+    // For non-LLM sites, try the normal permission flow
+    // (though it may not show UI in side panel context)
+    MediaCaptureDevicesDispatcher::GetInstance()->ProcessMediaAccessRequest(
+        web_contents, request, std::move(callback), /*extension=*/nullptr);
+  }
+}
+
+bool ThirdPartyLlmPanelCoordinator::CheckMediaAccessPermission(
+    content::RenderFrameHost* render_frame_host,
+    const url::Origin& security_origin,
+    blink::mojom::MediaStreamType type) {
+  // Check if this is a trusted LLM site using our existing provider list
+  const GURL origin_url = security_origin.GetURL();
+
+  // Check all LLM providers
+  for (const auto& provider : providers_) {
+    if (origin_url.host() == provider.url.host()) {
+      // Auto-grant permission check for trusted LLM sites
+      VLOG(2) << "[browseros] Media permission check for trusted LLM origin: "
+              << security_origin.Serialize()
+              << " type: " << static_cast<int>(type)
+              << " result: true (auto-granted)";
+      return true;
+    }
+  }
+
+  // For non-LLM sites, check Chrome's stored permissions
+  bool has_permission = MediaCaptureDevicesDispatcher::GetInstance()
+      ->CheckMediaAccessPermission(render_frame_host, security_origin,
+                                   type, /*extension=*/nullptr);
+
+  VLOG(2) << "[browseros] Media permission check for origin: "
+          << security_origin.Serialize()
+          << " type: " << static_cast<int>(type)
+          << " result: " << has_permission;
+
+  return has_permission;
+}
+
+void ThirdPartyLlmPanelCoordinator::CycleProvider() {
+  // If a provider change is already in flight, ignore additional toggle
+  // requests to prevent state races that could desynchronize the combobox and
+  // WebView.
+  if (provider_change_in_progress_)
+    return;
+
+  // Only cycle if the panel is showing (provider_selector_ exists when panel is open)
+  if (!provider_selector_) {
+    return;
+  }
+
+  if (providers_.empty()) {
+    return;
+  }
+
+  // Calculate next provider (cycle through all providers)
+  size_t next_provider_index = (current_provider_index_ + 1) % providers_.size();
+
+  // Update the provider selector
+  // Combobox selection changes made programmatically do NOT invoke the
+  // `SetCallback` observer, so we must call `OnProviderChanged()` manually
+  // to keep the page in sync with the visible provider label.
+  provider_selector_->SetSelectedIndex(next_provider_index);
+  OnProviderChanged();
+  
+  // Removed provider change notification to prevent crash
+}
+
+void ThirdPartyLlmPanelCoordinator::DidFinishLoad(
+    content::RenderFrameHost* render_frame_host,
+    const GURL& validated_url) {
+  // Nothing to do on page load
+}
+
+void ThirdPartyLlmPanelCoordinator::CleanupWebContents() {
+  // Cancel any pending timer callbacks first
+  if (feedback_timer_ && feedback_timer_->IsRunning()) {
+    feedback_timer_->Stop();
+  }
+
+  // Clear the WebView's association with WebContents
+  if (web_view_ && web_view_->web_contents()) {
+    web_view_->SetWebContents(nullptr);
+  }
+
+  // Destroy the WebContents we own
+  owned_web_contents_.reset();
+
+  // Stop observing
+  Observe(nullptr);
+}
+
+void ThirdPartyLlmPanelCoordinator::OnBrowserRemoved(Browser* browser) {
+  // Clean up WebContents when any browser is removed to be safe
+  // Since we no longer track the specific browser, clean up if profile matches
+  if (browser && browser->profile() == GetProfile()) {
+    CleanupWebContents();
+  }
+}
+
+void ThirdPartyLlmPanelCoordinator::OnProfileWillBeDestroyed(Profile* profile) {
+  if (profile == GetProfile()) {
+    // Profile is being destroyed - clean up WebContents if not already done
+    CleanupWebContents();
+  }
+}
+
+void ThirdPartyLlmPanelCoordinator::ShowOptionsMenu() {
+  if (!menu_button_) {
+    return;
+  }
+  
+  // Create menu model
+  menu_model_ = std::make_unique<ui::SimpleMenuModel>(this);
+  menu_model_->AddItemWithIcon(
+      IDC_COPY_CONTENT, 
+      u"Copy webpage to clipboard",
+      ui::ImageModel::FromVectorIcon(vector_icons::kContentCopyIcon));
+  menu_model_->AddItemWithIcon(
+      IDC_SCREENSHOT,
+      u"Screenshot webpage and copy",
+      ui::ImageModel::FromVectorIcon(vector_icons::kPhotoChromeRefreshIcon));
+  menu_model_->AddItemWithIcon(
+      IDC_REFRESH,
+      u"Reset LLM chat",
+      ui::ImageModel::FromVectorIcon(vector_icons::kReloadIcon));
+  menu_model_->AddItemWithIcon(
+      IDC_OPEN_IN_NEW_TAB,
+      u"Open in new tab",
+      ui::ImageModel::FromVectorIcon(vector_icons::kLaunchIcon));
+  menu_model_->AddSeparator(ui::NORMAL_SEPARATOR);
+  menu_model_->AddItemWithIcon(
+      IDC_CLASH_OF_GPTS,
+      u"Popout LLM Hub",
+      ui::ImageModel::FromVectorIcon(kTabGroupIcon));
+  
+  // Create and run menu
+  menu_runner_ = std::make_unique<views::MenuRunner>(
+      menu_model_.get(), views::MenuRunner::HAS_MNEMONICS);
+  menu_runner_->RunMenuAt(
+      menu_button_->GetWidget(),
+      nullptr,  // button controller
+      menu_button_->GetAnchorBoundsInScreen(),
+      views::MenuAnchorPosition::kTopRight,
+      ui::mojom::MenuSourceType::kNone);
+}
+
+void ThirdPartyLlmPanelCoordinator::ExecuteCommand(int command_id,
+                                                   int event_flags) {
+  std::string event_name;
+  switch (command_id) {
+    case IDC_COPY_CONTENT:
+      event_name = "llmchat.menu.content.copied";
+      break;
+    case IDC_SCREENSHOT:
+      event_name = "llmchat.menu.screenshot.captured";
+      break;
+    case IDC_REFRESH:
+      event_name = "llmchat.menu.refresh";
+      break;
+    case IDC_OPEN_IN_NEW_TAB:
+      event_name = "llmchat.menu.newtab";
+      break;
+    case IDC_CLASH_OF_GPTS:
+      event_name = "llmchat.menu.hub";
+      break;
+  }
+  if (!event_name.empty()) {
+    browseros_metrics::BrowserOSMetrics::Log(event_name);
+  }
+  
+  switch (command_id) {
+    case IDC_COPY_CONTENT:
+      OnCopyContent();
+      break;
+    case IDC_SCREENSHOT:
+      OnScreenshotContent();
+      break;
+    case IDC_REFRESH:
+      OnRefreshContent();
+      break;
+    case IDC_OPEN_IN_NEW_TAB:
+      OnOpenInNewTab();
+      break;
+    case IDC_CLASH_OF_GPTS:
+      if (Browser* browser = BrowserList::GetInstance()->GetLastActive()) {
+        if (auto* coordinator =
+                browser->browser_window_features()->clash_of_gpts_coordinator()) {
+          coordinator->Show();
+        }
+      }
+      break;
+  }
+}
+
+// static
+void ThirdPartyLlmPanelCoordinator::RegisterProfilePrefs(
+    user_prefs::PrefRegistrySyncable* registry) {
+  registry->RegisterListPref(kThirdPartyLlmProvidersPref);
+  registry->RegisterIntegerPref(kThirdPartyLlmSelectedProviderPref, 0);
+}
