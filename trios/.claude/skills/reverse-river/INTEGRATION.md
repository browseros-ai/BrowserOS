# Reverse River Integration Guide
## How to wire BrowserOSBridgeView into trios_app

### Step 1: Update main.swift
Add BrowserOS tab to QueenTabView:

```swift
// In QueenTabView.swift or main.swift where tabs are defined
Tab("BrowserOS", systemImage: "globe") {
    BrowserOSBridgeView()
}
```

### Step 2: Build
```bash
cd /Users/playra/BrowserOS-full/trios
swiftc -O -o trios_app \
  -framework SwiftUI -framework AppKit -framework WebKit -framework Combine \
  main.swift rings/SR-00/*.swift rings/SR-01/*.swift rings/SR-02/*.swift rings/SR-03/*.swift BR-OUTPUT/*.swift
```

### Step 3: Launch
```bash
./trios_app
```

### Step 4: Test
1. Open trios_app -> click status bar icon
2. Select "BrowserOS" tab
3. Type: "open google.com"
4. See BrowserOS navigate to Google
5. Results appear as native SwiftUI cards

### Architecture

```
trios SwiftUI -> BrowserOSBridgeView -> ChatViewModel+BrowserOS -> TriosMCPClient -> HTTP -> MCP Server (9105) -> BrowserOS Agent
```

### Files Created
- BR-OUTPUT/TriosMCPClient.swift - Actor for MCP HTTP
- BR-OUTPUT/ChatViewModel+BrowserOS.swift - ViewModel with reverse control
- BR-OUTPUT/BrowserOSBridgeView.swift - Native SwiftUI panel

### No .sh/.py Rule
All integration via Swift + HTTP only.
