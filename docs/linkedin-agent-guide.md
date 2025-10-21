# LinkedIn Job Automation Agent - User Guide

## Overview

The LinkedIn Job Automation Agent is a powerful AI-driven tool that helps automate your LinkedIn job search and application process using Perplexity AI. It can intelligently search for jobs, analyze job fit, and generate tailored application content.

## Features

- **Intelligent Job Search**: Uses Perplexity AI to optimize search queries and find relevant positions
- **Job Fit Analysis**: Analyzes job postings against your profile to provide match scores and recommendations
- **Automated Content Generation**: Creates personalized cover letters and application messages
- **Batch Processing**: Analyze multiple jobs at once to find the best matches
- **Real-time Web Search**: Leverages Perplexity's online search capabilities for up-to-date job market insights
- **Customizable**: Configure search parameters, matching criteria, and application preferences

## Getting Started

### Prerequisites

1. **Perplexity API Key**: You need a valid Perplexity API key
   - Sign up at [https://www.perplexity.ai/](https://www.perplexity.ai/)
   - Get your API key from the developer console
   - Set the API key in the configuration file or pass it when creating the agent

2. **LinkedIn Account**: You need to be logged into LinkedIn in your browser

3. **BrowserOS**: The agent runs within the BrowserOS environment

### Installation

The LinkedIn Agent is located in the BrowserOS AI side panel:

```
/packages/browseros/resources/files/ai_side_panel/
├── linkedin_agent.js           # Main agent class
├── linkedin_agent_config.js    # Configuration file
└── linkedin_agent_example.js   # Usage examples
```

### Quick Start

#### Method 1: Browser Console

1. Navigate to [LinkedIn Jobs](https://www.linkedin.com/jobs/)
2. Open the browser console (F12 or Cmd+Option+J)
3. Load the agent scripts:

```javascript
// The scripts should be loaded automatically by BrowserOS
// If not, you can load them manually:

// Create a new agent instance
const agent = new LinkedInAgent({
  apiKey: 'YOUR_PERPLEXITY_API_KEY' // Replace with your actual API key
});

// Initialize the agent
await agent.initialize();

// Extract jobs from current page
const jobs = await agent.extractJobListings();
console.log(`Found ${jobs.length} jobs`);
```

#### Method 2: Using Configuration

```javascript
// Load pre-configured settings
const config = getLinkedInAgentConfig();

// Update your profile
updateUserProfile({
  name: 'Your Name',
  currentTitle: 'Software Engineer',
  yearsOfExperience: 5,
  technicalSkills: ['JavaScript', 'React', 'Node.js', 'AWS'],
  targetRoles: ['Senior Software Engineer', 'Full Stack Engineer']
});

// Create agent with config
const agent = new LinkedInAgent({
  apiKey: config.perplexity.apiKey,
  model: config.perplexity.model
});
```

## Usage Examples

### Example 1: Search for Jobs

```javascript
const searchParams = {
  keywords: 'Software Engineer',
  location: 'San Francisco, CA',
  jobType: 'full-time',
  experienceLevel: 'mid-senior',
  remote: true
};

const userProfile = {
  currentTitle: 'Senior Software Engineer',
  yearsOfExperience: 5,
  technicalSkills: ['JavaScript', 'Python', 'React', 'AWS']
};

const results = await agent.runJobSearchWorkflow(searchParams, userProfile);
console.log(results);
```

### Example 2: Analyze Job Fit

```javascript
const job = {
  title: 'Senior Backend Engineer',
  company: 'Tech Company',
  location: 'Remote',
  description: 'Looking for 5+ years experience in Node.js...'
};

const analysis = await agent.analyzeJobFit(job, userProfile);
console.log('Match Score:', analysis.matchScore);
console.log('Recommendation:', analysis.recommendation);
```

### Example 3: Generate Cover Letter

```javascript
const coverLetter = await agent.generateApplicationContent(
  job,
  userProfile,
  analysis
);
console.log(coverLetter);
```

### Example 4: Extract and Analyze Current Page

```javascript
// Navigate to a LinkedIn job search page, then run:

const jobs = await agent.extractJobListings();
console.log(`Found ${jobs.length} jobs`);

// Analyze each job
for (const job of jobs.slice(0, 5)) {
  const analysis = await agent.analyzeJobFit(job, userProfile);
  console.log(`${job.title}: ${analysis.matchScore}% match`);

  // Wait between requests to avoid rate limiting
  await new Promise(r => setTimeout(r, 2000));
}
```

## Configuration

### API Configuration

Edit `linkedin_agent_config.js` to customize:

```javascript
perplexity: {
  apiKey: 'your-api-key',
  model: 'llama-3.1-sonar-large-128k-online',
  // Available models:
  // - llama-3.1-sonar-small-128k-online (faster, cheaper)
  // - llama-3.1-sonar-large-128k-online (balanced)
  // - llama-3.1-sonar-huge-128k-online (most powerful)
}
```

### Search Defaults

```javascript
search: {
  maxResults: 20,
  defaultLocation: 'United States',
  defaultJobType: 'full-time',
  includeRemote: true,
  searchRecency: 'week'
}
```

### Application Settings

```javascript
application: {
  autoApply: false,              // Enable automatic applications
  maxApplicationsPerSession: 10, // Limit applications per run
  minMatchScore: 70,             // Only apply to 70%+ matches
  reviewBeforeApply: true        // Review before submitting
}
```

### User Profile

Customize your profile in the config:

```javascript
userProfile: {
  name: 'John Doe',
  currentTitle: 'Software Engineer',
  yearsOfExperience: 5,
  technicalSkills: ['JavaScript', 'React', 'Node.js'],
  experience: [
    {
      title: 'Senior Engineer',
      company: 'Tech Co',
      achievements: ['Led team', 'Shipped features']
    }
  ]
}
```

## Advanced Features

### Perplexity Search Options

The agent uses Perplexity's advanced search capabilities:

```javascript
const result = await agent.queryPerplexity(prompt, {
  searchDomainFilter: ['linkedin.com'],  // Filter to specific domains
  searchRecencyFilter: 'week',           // Recent results only
  returnCitations: true,                 // Get sources
  temperature: 0.2                       // Lower = more focused
});
```

### Custom Job Analysis

You can customize the analysis criteria:

```javascript
const analysis = await agent.analyzeJobFit(job, {
  ...userProfile,
  priorities: {
    salary: 'high',
    remote: 'required',
    techStack: ['React', 'Node.js'],
    companySize: 'startup'
  }
});
```

### Batch Processing

Process multiple jobs efficiently:

```javascript
const jobs = await agent.extractJobListings();
const analyses = [];

for (const job of jobs) {
  const analysis = await agent.analyzeJobFit(job, userProfile);
  if (analysis.matchScore >= 70) {
    analyses.push({ job, analysis });
  }
  await new Promise(r => setTimeout(r, 1500)); // Rate limiting
}

// Sort by match score
analyses.sort((a, b) => b.analysis.matchScore - a.analysis.matchScore);
```

## API Reference

### LinkedInAgent Class

#### Constructor

```javascript
new LinkedInAgent(config)
```

**Parameters:**
- `config.apiKey` (string): Perplexity API key
- `config.model` (string): Perplexity model to use
- `config.autoApply` (boolean): Enable auto-application
- `config.maxApplications` (number): Max applications per session

#### Methods

##### `initialize()`
Initialize and verify API connection.

```javascript
await agent.initialize();
```

##### `searchJobs(searchParams)`
Search for jobs with specified parameters.

```javascript
const jobs = await agent.searchJobs({
  keywords: 'Engineer',
  location: 'SF',
  remote: true
});
```

##### `extractJobListings()`
Extract jobs from the current LinkedIn page.

```javascript
const jobs = await agent.extractJobListings();
```

##### `analyzeJobFit(job, userProfile)`
Analyze how well a job matches your profile.

```javascript
const analysis = await agent.analyzeJobFit(job, profile);
// Returns: { matchScore, recommendation, reasons, talkingPoints }
```

##### `generateApplicationContent(job, userProfile, analysis)`
Generate personalized cover letter or message.

```javascript
const content = await agent.generateApplicationContent(job, profile, analysis);
```

##### `runJobSearchWorkflow(searchParams, userProfile)`
Complete workflow: search, analyze, and prepare applications.

```javascript
const results = await agent.runJobSearchWorkflow(params, profile);
```

## Best Practices

### 1. Rate Limiting

Always add delays between API calls to avoid rate limits:

```javascript
await new Promise(resolve => setTimeout(resolve, 2000));
```

### 2. Profile Accuracy

Keep your user profile detailed and up-to-date for better matching:

```javascript
userProfile: {
  // Be specific about your skills and experience
  technicalSkills: ['React', 'Node.js', 'PostgreSQL'],
  // Include quantifiable achievements
  achievements: ['Reduced load time by 40%', 'Led team of 5']
}
```

### 3. Review Before Applying

Always review generated content before submitting:

```javascript
application: {
  reviewBeforeApply: true,
  autoApply: false  // Manual review recommended
}
```

### 4. Use Specific Search Terms

More specific searches yield better results:

```javascript
searchParams: {
  keywords: 'Senior React Engineer SaaS',  // Specific
  // vs
  keywords: 'Engineer'  // Too broad
}
```

### 5. Monitor API Usage

Perplexity API has usage limits. Monitor your consumption:

```javascript
const result = await agent.queryPerplexity(prompt);
console.log('Tokens used:', result.usage);
```

## Troubleshooting

### API Key Issues

```javascript
// Verify API key is set
console.log(agent.getStatus());

// Update API key
setPerplexityApiKey('your-new-key');
```

### LinkedIn Page Structure Changes

If job extraction fails, LinkedIn may have changed their HTML structure:

```javascript
// Check what elements are on the page
document.querySelectorAll('[class*="job"]').forEach(el => {
  console.log(el.className);
});
```

### Rate Limiting

If you hit rate limits, increase delays:

```javascript
advanced: {
  delayBetweenActions: 3000,  // Increase from 2000
  delayBetweenApplications: 10000
}
```

## Privacy & Security

### Data Handling

- **API Key**: Stored locally in the configuration file
- **Profile Data**: Only used for job matching, not transmitted elsewhere
- **Job Data**: Extracted from LinkedIn, processed by Perplexity API
- **Generated Content**: Created locally, you control when to submit

### Recommendations

1. **Don't commit API keys** to version control
2. **Review permissions** before enabling auto-apply
3. **Use environment variables** for sensitive data in production
4. **Monitor activity** to ensure compliance with LinkedIn's terms

## Limitations

1. **Manual Application Required**: The agent generates content but doesn't automatically submit applications (requires additional LinkedIn API integration)
2. **Rate Limits**: Both Perplexity API and LinkedIn have rate limits
3. **Page Structure**: LinkedIn's HTML structure may change, affecting job extraction
4. **No Form Filling**: The agent doesn't automatically fill application forms
5. **Authentication**: Requires active LinkedIn session

## Future Enhancements

Planned features:

- [ ] Integration with LinkedIn API for direct job posting access
- [ ] Resume parsing and matching
- [ ] Application tracking dashboard
- [ ] Email notifications for high-match jobs
- [ ] Calendar integration for interview scheduling
- [ ] Multi-platform support (Indeed, Glassdoor, etc.)
- [ ] Browser extension for one-click job analysis

## Support

For issues or questions:

1. Check the [BrowserOS Documentation](https://docs.browseros.com)
2. Review example code in `linkedin_agent_example.js`
3. File issues on the BrowserOS GitHub repository

## License

Part of BrowserOS - see main project license.

## Credits

- **Perplexity AI**: Powers intelligent job analysis and content generation
- **BrowserOS**: Provides the browser automation framework
- **Community**: Thanks to all contributors

---

**Happy Job Hunting!** 🚀
