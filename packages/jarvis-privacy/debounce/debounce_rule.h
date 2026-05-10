// Copyright (c) 2021 The Brave Authors. All rights reserved.
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this file,
// You can obtain one at https://mozilla.org/MPL/2.0/.

#ifndef BRAVE_COMPONENTS_DEBOUNCE_CORE_BROWSER_DEBOUNCE_RULE_H_
#define BRAVE_COMPONENTS_DEBOUNCE_CORE_BROWSER_DEBOUNCE_RULE_H_

#include <memory>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

#include "base/containers/flat_set.h"
#include "base/json/json_value_converter.h"
#include "base/strings/escape.h"
#include "base/types/expected.h"
#include "components/prefs/pref_service.h"
#include "extensions/common/url_pattern_set.h"

class GURL;

namespace base {
class Value;
}  // namespace base

namespace debounce {

enum DebounceAction {
  kDebounceNoAction,
  kDebounceRedirectToParam,
  kDebounceRegexPath,
  kDebounceBase64DecodeAndRedirectToParam,
  kDebounceRegexPathTemplate
};

enum DebouncePrependScheme {
  kDebounceNoSchemePrepend,
  kDebounceSchemePrependHttp,
  kDebounceSchemePrependHttps
};

class DebounceRule {
 public:
  DebounceRule();
  ~DebounceRule();

  // Registers the mapping between JSON field names and the members in this
  // class.
  static void RegisterJSONConverter(
      base::JSONValueConverter<DebounceRule>* converter);
  static bool ParseDebounceAction(std::string_view value,
                                  DebounceAction* field);
  static bool ParsePrependScheme(std::string_view value,
                                 DebouncePrependScheme* field);
  static base::expected<std::pair<std::vector<std::unique_ptr<DebounceRule>>,
                                  base::flat_set<std::string>>,
                        std::string>
  ParseRules(std::string_view contents);
  static const std::string GetETLDForDebounce(std::string_view host);
  static bool IsSameETLDForDebounce(const GURL& url1, const GURL& url2);
  static bool GetURLPatternSetFromValue(const base::Value* value,
                                        extensions::URLPatternSet* result);

  bool Apply(const GURL& original_url,
             GURL* final_url,
             const PrefService* prefs) const;
  const extensions::URLPatternSet& include_pattern_set() const {
    return include_pattern_set_;
  }

 private:
  bool CheckPrefForRule(const PrefService* prefs) const;
  bool ValidateAndParsePatternRegex(
      std::string_view pattern,
      std::string_view path,
      std::vector<std::string>& captured_groups) const;
  extensions::URLPatternSet include_pattern_set_;
  extensions::URLPatternSet exclude_pattern_set_;
  DebounceAction action_;
  DebouncePrependScheme prepend_scheme_;
  std::string param_;
  std::string pref_;
  std::string redirect_url_template_;
};

}  // namespace debounce

#endif  // BRAVE_COMPONENTS_DEBOUNCE_CORE_BROWSER_DEBOUNCE_RULE_H_
