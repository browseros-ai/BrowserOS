export const getFavicons = (host: string) => {
  // Security Hardening: Disabled Google Favicon service to prevent navigation history leakage
  // Using a generic fallback icon
  return '/icons/generic-favicon.png'
}
