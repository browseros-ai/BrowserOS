/** Small centered BrowserClaw mark shown above the idle omnibox. */
export function BrandMark() {
  return (
    <div className="flex justify-center">
      <img
        src="/icons/browserclaw.svg"
        alt="BrowserClaw"
        className="size-9 rounded-lg shadow-card"
      />
    </div>
  )
}
