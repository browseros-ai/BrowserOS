# API Key Setup Instructions

## Setting Up Your Perplexity API Key

The LinkedIn Agent requires a Perplexity API key to function. Here's how to set it up:

### Option 1: Direct Configuration (For Testing)

Edit `linkedin_agent_config.js` and replace the empty string with your API key:

```javascript
perplexity: {
  apiKey: 'pplx-YOUR-API-KEY-HERE',
  baseUrl: 'https://api.perplexity.ai',
  model: 'llama-3.1-sonar-large-128k-online'
}
```

**Note**: This method stores the key in plaintext. Do NOT commit this file to version control.

### Option 2: Environment Variable (Recommended)

1. Create a `.env` file in the `ai_side_panel` directory:

```bash
cp .env.example .env
```

2. Edit `.env` and add your API key:

```
PERPLEXITY_API_KEY=pplx-YOUR-API-KEY-HERE
```

3. The configuration will automatically load it:

```javascript
apiKey: process.env.PERPLEXITY_API_KEY || ''
```

### Option 3: Pass Directly When Creating Agent

When initializing the agent in your code, pass the API key directly:

```javascript
const agent = new LinkedInAgent({
  apiKey: 'pplx-YOUR-API-KEY-HERE'
});
```

## Getting a Perplexity API Key

1. Visit [https://www.perplexity.ai/](https://www.perplexity.ai/)
2. Sign up for an account
3. Navigate to the API section or developer console
4. Generate a new API key
5. Copy the key (it will start with `pplx-`)

## Using Your API Key

Once you have your Perplexity API key, you can use it in several ways:

### Quick Setup (Browser Console)

```javascript
// Direct usage
const agent = new LinkedInAgent({
  apiKey: 'pplx-YOUR-API-KEY-HERE'
});
```

### Update Configuration File

Edit `linkedin_agent_config.js`:

```javascript
perplexity: {
  apiKey: 'pplx-YOUR-API-KEY-HERE',
  // ... rest of config
}
```

**Note**: If you received an API key separately (via email, documentation, or from the project creator), replace `pplx-YOUR-API-KEY-HERE` with that key.

## Security Best Practices

1. **Never commit API keys** to version control
2. **Use environment variables** for production
3. **Rotate keys regularly** if they're exposed
4. **Monitor usage** on the Perplexity dashboard
5. **Set up rate limits** to prevent unexpected charges

## Troubleshooting

### "API key not found" error

```javascript
// Check if key is loaded
const config = getLinkedInAgentConfig();
console.log('API Key loaded:', config.perplexity.apiKey ? 'Yes' : 'No');

// Set key manually
setPerplexityApiKey('pplx-YOUR-KEY-HERE');
```

### "Unauthorized" error

- Verify the API key is correct
- Check if the key is active in your Perplexity dashboard
- Ensure there are no extra spaces or quotes

### "Rate limit exceeded"

- Wait a few minutes before trying again
- Reduce the number of concurrent requests
- Consider upgrading your Perplexity plan

## Support

For API-related issues:
- Perplexity Support: https://www.perplexity.ai/support
- BrowserOS Issues: https://github.com/browseros/browseros/issues
