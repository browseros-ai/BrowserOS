# Integration tests

From the SDK worktree root:

```sh
cd packages/browseros-agent
export NEO_E2E_BROWSER="/Applications/BrowserOS neo.app/Contents/MacOS/BrowserOS neo"
```

Headless:

```sh
NEO_E2E_HEADED=0 cargo test -p neo-runtime \
  --test amazon_cart_live --test hacker_news_live -- --ignored --nocapture
```

Headed (visible browser):

```sh
NEO_E2E_HEADED=1 cargo test -p neo-runtime \
  --test amazon_cart_live --test hacker_news_live -- --ignored --nocapture
```
