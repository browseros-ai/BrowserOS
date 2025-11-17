/**
 * Validates and normalizes URLs for the browser
 */
export class URLValidator {
  private static readonly URL_REGEX =
    /^https?:\/\/(www\.)?[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b([-a-zA-Z0-9()@:%_\+.~#?&//=]*)$/;

  private static readonly DOMAIN_REGEX =
    /^[-a-zA-Z0-9@:%._\+~#=]{1,256}\.[a-zA-Z0-9()]{1,6}\b([-a-zA-Z0-9()@:%_\+.~#?&//=]*)$/;

  /**
   * Check if a string is a valid URL
   */
  static isValidURL(url: string): boolean {
    try {
      new URL(url);
      return true;
    } catch {
      return false;
    }
  }

  /**
   * Check if a string looks like a domain
   */
  static isDomain(input: string): boolean {
    return this.DOMAIN_REGEX.test(input) || input.includes('localhost');
  }

  /**
   * Normalize a user input to a valid URL
   * - Add https:// if missing
   * - Convert search queries to Google search
   * - Handle localhost and IP addresses
   */
  static normalizeInput(input: string): string {
    const trimmed = input.trim();

    // Empty input
    if (!trimmed) {
      return 'https://www.google.com';
    }

    // Already a valid URL
    if (this.isValidURL(trimmed)) {
      return trimmed;
    }

    // Has protocol but invalid
    if (trimmed.startsWith('http://') || trimmed.startsWith('https://')) {
      return trimmed;
    }

    // Localhost
    if (trimmed.startsWith('localhost')) {
      return `http://${trimmed}`;
    }

    // IP address (simple check)
    if (/^\d{1,3}\.\d{1,3}\.\d{1,3}\.\d{1,3}/.test(trimmed)) {
      return `http://${trimmed}`;
    }

    // Looks like a domain
    if (this.isDomain(trimmed)) {
      return `https://${trimmed}`;
    }

    // Treat as search query
    return `https://www.google.com/search?q=${encodeURIComponent(trimmed)}`;
  }

  /**
   * Extract domain from URL
   */
  static getDomain(url: string): string | null {
    try {
      const urlObj = new URL(url);
      return urlObj.hostname;
    } catch {
      return null;
    }
  }

  /**
   * Get favicon URL for a domain
   */
  static getFaviconURL(url: string): string {
    const domain = this.getDomain(url);
    if (!domain) {
      return 'https://www.google.com/favicon.ico';
    }
    return `https://www.google.com/s2/favicons?domain=${domain}&sz=32`;
  }

  /**
   * Check if URL is secure (HTTPS)
   */
  static isSecure(url: string): boolean {
    try {
      const urlObj = new URL(url);
      return urlObj.protocol === 'https:';
    } catch {
      return false;
    }
  }

  /**
   * Shorten URL for display
   */
  static shortenURL(url: string, maxLength: number = 50): string {
    if (url.length <= maxLength) {
      return url;
    }

    try {
      const urlObj = new URL(url);
      const domain = urlObj.hostname;
      const path = urlObj.pathname + urlObj.search;

      if (domain.length >= maxLength - 3) {
        return domain.substring(0, maxLength - 3) + '...';
      }

      const remainingLength = maxLength - domain.length - 3;
      const shortenedPath = path.substring(0, remainingLength);

      return `${domain}${shortenedPath}...`;
    } catch {
      return url.substring(0, maxLength - 3) + '...';
    }
  }

  /**
   * Check if URL is a search engine
   */
  static isSearchEngine(url: string): boolean {
    const searchEngines = [
      'google.com',
      'bing.com',
      'duckduckgo.com',
      'yahoo.com',
      'baidu.com',
    ];

    const domain = this.getDomain(url);
    if (!domain) return false;

    return searchEngines.some((engine) => domain.includes(engine));
  }

  /**
   * Extract search query from search engine URL
   */
  static extractSearchQuery(url: string): string | null {
    try {
      const urlObj = new URL(url);

      // Google
      if (urlObj.hostname.includes('google.com')) {
        return urlObj.searchParams.get('q');
      }

      // Bing
      if (urlObj.hostname.includes('bing.com')) {
        return urlObj.searchParams.get('q');
      }

      // DuckDuckGo
      if (urlObj.hostname.includes('duckduckgo.com')) {
        return urlObj.searchParams.get('q');
      }

      return null;
    } catch {
      return null;
    }
  }
}
