# RFC: AI Ghost Mode — Invisible Agent Learning

**Issue:** [#336](https://github.com/browseros-ai/BrowserOS/issues/336)  
**Status:** Draft  
**Author:** Community Contributor  
**Created:** 2026-01-26  

---

## 1. Executive Summary

AI Ghost Mode enables BrowserOS to passively observe user browsing patterns and suggest automations — without requiring users to describe tasks or write prompts. The browser learns from successful human actions and generates Workflow graphs automatically.

**One-liner:** *"BrowserOS learns how YOU browse, then does it for you — privately, automatically, invisibly."*

---

## 2. Goals & Non-Goals

### Goals
- [ ] Passively record user actions (clicks, keystrokes, navigation) locally
- [ ] Detect repetitive patterns across browsing sessions
- [ ] Suggest "Automate This" when patterns are detected
- [ ] One-click conversion to existing Workflow graph format
- [ ] Execute learned automations in background tabs (Ghost Mode)
- [ ] 100% local, privacy-first — no data leaves device

### Non-Goals
- Cloud-based pattern detection
- Recording sensitive inputs (passwords, credit cards)
- Replacing manual Workflow creation (this complements it)
- Cross-device sync of learned patterns

---

## 3. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              USER BROWSING                                   │
│                                    │                                         │
│                                    ▼                                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                        ACTION RECORDER                                │   │
│  │  • Captures: clicks, keystrokes, navigation, form fills              │   │
│  │  • Filters: excludes passwords, sensitive fields                     │   │
│  │  • Location: chrome/browser/browseros/ghost_mode/action_recorder.cc  │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                    │                                         │
│                                    ▼                                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                        ACTION STORE                                   │   │
│  │  • Local SQLite database in user profile                             │   │
│  │  • Stores: action sequences with timestamps, selectors, URLs         │   │
│  │  • Retention: 30 days rolling window                                 │   │
│  │  • Location: chrome/browser/browseros/ghost_mode/action_store.cc     │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                    │                                         │
│                                    ▼                                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                      PATTERN DETECTOR                                 │   │
│  │  • Runs periodically (every 6 hours) or on-demand                    │   │
│  │  • Detects: repeated action sequences (≥3 occurrences)               │   │
│  │  • Algorithm: Sequence alignment + fuzzy matching                    │   │
│  │  • Location: chrome/browser/browseros/ghost_mode/pattern_detector.cc │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                    │                                         │
│                                    ▼                                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                      SUGGESTION ENGINE                                │   │
│  │  • Presents non-intrusive notification when pattern found            │   │
│  │  • Shows: "I noticed you do this often. Automate it?"                │   │
│  │  • User can: Accept / Dismiss / Never show for this pattern          │   │
│  │  • Location: chrome/browser/browseros/ghost_mode/suggestion_ui.cc    │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                    │                                         │
│                                    ▼                                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                     WORKFLOW GENERATOR                                │   │
│  │  • Converts action sequence → Workflow graph JSON                    │   │
│  │  • Integrates with existing Workflows feature                        │   │
│  │  • User can edit before saving                                       │   │
│  │  • Location: chrome/browser/browseros/ghost_mode/workflow_gen.cc     │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                    │                                         │
│                                    ▼                                         │
│  ┌──────────────────────────────────────────────────────────────────────┐   │
│  │                      GHOST EXECUTOR                                   │   │
│  │  • Runs saved Workflows in background tab (invisible)                │   │
│  │  • Shows ghost indicator in toolbar during execution                 │   │
│  │  • Can be triggered: manually, on schedule, or auto (on page visit)  │   │
│  │  • Location: chrome/browser/browseros/ghost_mode/ghost_executor.cc   │   │
│  └──────────────────────────────────────────────────────────────────────┘   │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Implementation Plan

### Phase 1: Action Recording Infrastructure (Week 1-2)

| Task | Description | Files |
|------|-------------|-------|
| **1.1** | Create ghost_mode directory structure | `chrome/browser/browseros/ghost_mode/BUILD.gn` |
| **1.2** | Implement Action data model | `ghost_mode_types.h` |
| **1.3** | Implement ActionRecorder class | `action_recorder.cc/h` |
| **1.4** | Hook into WebContents events | Integration with existing `browser_os_change_detector.cc` |
| **1.5** | Implement ActionStore (SQLite) | `action_store.cc/h` |
| **1.6** | Add prefs for Ghost Mode toggle | `ghost_mode_prefs.cc/h` |
| **1.7** | Add settings UI toggle | Settings page integration |

### Phase 2: Pattern Detection (Week 3-4)

| Task | Description | Files |
|------|-------------|-------|
| **2.1** | Design pattern matching algorithm | Algorithm doc |
| **2.2** | Implement sequence alignment | `pattern_detector.cc/h` |
| **2.3** | Implement fuzzy URL/selector matching | `pattern_matcher.cc/h` |
| **2.4** | Create background task scheduler | `pattern_scheduler.cc/h` |
| **2.5** | Add pattern confidence scoring | Integration with detector |

### Phase 3: Suggestion UI (Week 5)

| Task | Description | Files |
|------|-------------|-------|
| **3.1** | Design notification UI (non-intrusive) | Figma/mockups |
| **3.2** | Implement suggestion InfoBar | `ghost_suggestion_infobar.cc/h` |
| **3.3** | Handle user responses (Accept/Dismiss/Block) | Event handlers |
| **3.4** | Store dismissed patterns to avoid re-suggesting | Prefs integration |

### Phase 4: Workflow Generation (Week 6)

| Task | Description | Files |
|------|-------------|-------|
| **4.1** | Design action-to-workflow mapping | Schema doc |
| **4.2** | Implement WorkflowGenerator | `workflow_generator.cc/h` |
| **4.3** | Integrate with existing Workflows UI | Extension messaging |
| **4.4** | Add "Edit before saving" flow | UI integration |

### Phase 5: Ghost Executor (Week 7-8)

| Task | Description | Files |
|------|-------------|-------|
| **5.1** | Implement background tab creation | `ghost_executor.cc/h` |
| **5.2** | Add toolbar ghost indicator | Toolbar integration |
| **5.3** | Implement execution progress tracking | Progress UI |
| **5.4** | Add execution triggers (manual/schedule/auto) | Trigger system |
| **5.5** | Error handling and retry logic | Error handling |

### Phase 6: Privacy & Polish (Week 9)

| Task | Description | Files |
|------|-------------|-------|
| **6.1** | Implement sensitive field detection | `sensitive_detector.cc/h` |
| **6.2** | Add data retention controls | Settings UI |
| **6.3** | Create "What Ghost Mode Learned" dashboard | New WebUI page |
| **6.4** | Add export/delete all data options | Privacy controls |
| **6.5** | Write user documentation | Docs site |

---

## 5. Detailed Component Specifications

### 5.1 Action Data Model

```cpp
// ghost_mode_types.h

namespace browseros::ghost_mode {

enum class ActionType {
  kClick,
  kType,
  kNavigate,
  kScroll,
  kSelect,
  kSubmit,
  kKeyPress,
};

struct RecordedAction {
  ActionType type;
  std::string url_pattern;          // Normalized URL (no query params for matching)
  std::string selector;             // CSS selector or accessibility label
  std::string value;                // For type actions (ENCRYPTED if sensitive)
  base::Time timestamp;
  int tab_id;
  std::string session_id;           // Groups actions in same browsing session
  base::Value::Dict metadata;       // Extra context (viewport size, etc.)
};

struct ActionSequence {
  std::string id;                   // UUID
  std::vector<RecordedAction> actions;
  int occurrence_count;
  base::Time first_seen;
  base::Time last_seen;
  double confidence_score;          // 0.0 - 1.0
  std::string suggested_name;       // AI-generated name for the workflow
};

}  // namespace browseros::ghost_mode
```

### 5.2 Pattern Detection Algorithm

```
ALGORITHM: Detect Repeated Action Sequences

INPUT: List of all recorded actions from last 30 days
OUTPUT: List of ActionSequence with occurrence_count >= 3

1. Group actions by session_id
2. For each unique starting action (navigation to a URL):
   a. Extract subsequences of length 3-20 actions
   b. Normalize subsequences:
      - URLs: strip query params, use domain + path pattern
      - Selectors: use stable selectors (id, data-testid, aria-label)
      - Values: hash or placeholder (don't match on actual input values)
3. Hash each normalized subsequence
4. Count occurrences of each hash
5. For hashes with count >= 3:
   a. Retrieve original sequences
   b. Calculate confidence score based on:
      - Consistency of timing between actions
      - Selector stability (did selectors change?)
      - Success rate (did sequence complete each time?)
   c. Generate suggested name using simple heuristics or local LLM
6. Return sequences sorted by (occurrence_count * confidence_score)
```

### 5.3 Sensitive Field Detection

Fields to NEVER record:
- `input[type="password"]`
- `input[autocomplete="cc-number"]` (credit card)
- `input[autocomplete="cc-cvc"]`
- `input[name*="ssn"]` (social security)
- `input[name*="password"]`
- `input[name*="secret"]`
- `input[name*="token"]`
- Fields within `.password-form`, `#login-form`, etc.

Implementation: Check these patterns BEFORE recording, never store.

### 5.4 Extension API

```idl
// chrome/common/extensions/api/ghost_mode.idl

namespace ghostMode {
  dictionary Pattern {
    DOMString id;
    DOMString name;
    long occurrenceCount;
    double confidenceScore;
    DOMString[] actionSummary;  // Human-readable summary
  };

  callback PatternsCallback = void(Pattern[] patterns);
  callback ConvertCallback = void(DOMString workflowJson);

  interface Functions {
    // Get all detected patterns
    static void getPatterns(PatternsCallback callback);
    
    // Convert a pattern to workflow
    static void convertToWorkflow(DOMString patternId, ConvertCallback callback);
    
    // Dismiss a pattern (don't suggest again)
    static void dismissPattern(DOMString patternId);
    
    // Enable/disable recording
    static void setRecordingEnabled(boolean enabled);
    
    // Get recording status
    static void getRecordingEnabled(BooleanCallback callback);
    
    // Clear all recorded data
    static void clearAllData(VoidCallback callback);
  };
};
```

---

## 6. Settings UI

Add to `chrome://browseros/settings`:

```
┌─────────────────────────────────────────────────────────────────┐
│  Ghost Mode                                              [ON]   │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ○ Learn from my browsing to suggest automations               │
│    BrowserOS observes your actions locally to detect           │
│    repetitive tasks. No data leaves your device.               │
│                                                                 │
│  Data Retention: [30 days ▾]                                   │
│                                                                 │
│  [View Learned Patterns]    [Clear All Data]                   │
│                                                                 │
│  ─────────────────────────────────────────────────────────     │
│                                                                 │
│  Excluded Sites:                                                │
│  + Add site to exclude from learning                           │
│                                                                 │
│  • bank.example.com                              [Remove]      │
│  • healthcare.example.com                        [Remove]      │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 7. File Structure

```
chrome/browser/browseros/ghost_mode/
├── BUILD.gn
├── ghost_mode_types.h
├── ghost_mode_prefs.cc
├── ghost_mode_prefs.h
├── action_recorder.cc
├── action_recorder.h
├── action_store.cc
├── action_store.h
├── pattern_detector.cc
├── pattern_detector.h
├── pattern_matcher.cc
├── pattern_matcher.h
├── pattern_scheduler.cc
├── pattern_scheduler.h
├── sensitive_detector.cc
├── sensitive_detector.h
├── suggestion_controller.cc
├── suggestion_controller.h
├── workflow_generator.cc
├── workflow_generator.h
├── ghost_executor.cc
├── ghost_executor.h
└── resources/
    └── ghost_mode_strings.grdp
```

---

## 8. Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| **Privacy concerns** | All processing local, clear documentation, easy data deletion |
| **Performance impact** | Throttle recording, use background thread, limit storage size |
| **Pattern false positives** | High confidence threshold (0.8), user can dismiss |
| **Selector instability** | Use multiple selector strategies, validate before execution |
| **User confusion** | Clear UI, explicit opt-in, easy to disable |

---

## 9. Success Metrics

- **Adoption:** % of users with Ghost Mode enabled (opt-in)
- **Detection Rate:** Patterns detected per active user per week
- **Conversion Rate:** Patterns → Workflows created
- **Execution Success:** % of Ghost Mode executions that complete successfully
- **Retention:** Users still using Ghost Mode after 30 days

---

## 10. Open Questions

1. Should Ghost Mode be opt-in (default off) or opt-out (default on)?
   - **Recommendation:** Opt-in with prominent onboarding prompt

2. Should we use local LLM for pattern naming, or simple heuristics?
   - **Recommendation:** Start with heuristics, add LLM later

3. How do we handle dynamic sites (SPAs) where selectors change?
   - **Recommendation:** Multiple selector strategies + accessibility tree

4. Should Ghost Mode work in Incognito?
   - **Recommendation:** No, disable by default (privacy expectation)

---

## 11. Timeline

| Week | Milestone |
|------|-----------|
| 1-2 | Action Recording Infrastructure |
| 3-4 | Pattern Detection |
| 5 | Suggestion UI |
| 6 | Workflow Generation |
| 7-8 | Ghost Executor |
| 9 | Privacy & Polish |
| 10 | Testing & Bug Fixes |
| 11 | Documentation & Beta Release |

**Total:** ~11 weeks to MVP

---

## 12. Related Issues

- [#329](https://github.com/browseros-ai/BrowserOS/issues/329) - RFC: Making browser agents reliable
- [#185](https://github.com/browseros-ai/BrowserOS/issues/185) - AI Cursor in-place edits
- [#317](https://github.com/browseros-ai/BrowserOS/issues/317) - Use ChatGPT/Gemini without API key
- [#324](https://github.com/browseros-ai/BrowserOS/issues/324) - Display AI thinking process

---

## Appendix A: Example User Flow

```
Day 1:
  User fills out a expense report form on company.example.com
  → Ghost Mode records: navigate → click → type → click → type → submit

Day 3:
  User fills out same form again
  → Ghost Mode records sequence, notes similarity to Day 1

Day 5:
  User fills out form third time
  → Ghost Mode detects pattern (3 occurrences, 0.92 confidence)
  → Shows subtle notification: "I noticed you fill out expense reports often. Automate it?"

User clicks "Automate This":
  → Workflow graph generated from recorded sequence
  → User reviews and edits (can parameterize amount, date fields)
  → Saves as "Weekly Expense Report"

Day 8:
  User visits company.example.com
  → Ghost Mode suggests: "Run 'Weekly Expense Report'?"
  → User clicks yes
  → Workflow executes in background tab (ghost indicator shows in toolbar)
  → Notification: "Expense report submitted successfully ✓"
```

---

*This RFC is a living document. Please comment on the GitHub issue with feedback!*
