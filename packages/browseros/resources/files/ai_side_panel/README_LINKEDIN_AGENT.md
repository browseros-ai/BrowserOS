# LinkedIn Job Automation Agent

AI-powered LinkedIn job search and application automation using Perplexity AI.

## Quick Start

### 1. Navigate to LinkedIn

Open [LinkedIn Jobs](https://www.linkedin.com/jobs/) in BrowserOS.

### 2. Open Browser Console

Press `F12` or `Cmd+Option+J` to open the developer console.

### 3. Initialize the Agent

```javascript
// Create agent instance
const agent = new LinkedInAgent({
  apiKey: 'YOUR_PERPLEXITY_API_KEY' // Replace with your actual API key
});

// Initialize
await agent.initialize();

// Extract jobs from current page
const jobs = await agent.extractJobListings();
console.log(`Found ${jobs.length} jobs`);
```

### 4. Analyze Jobs

```javascript
// Define your profile
const myProfile = {
  currentTitle: 'Software Engineer',
  yearsOfExperience: 5,
  technicalSkills: ['JavaScript', 'React', 'Node.js', 'Python'],
  targetRoles: ['Senior Software Engineer', 'Full Stack Engineer']
};

// Analyze first job
const job = jobs[0];
const analysis = await agent.analyzeJobFit(job, myProfile);

console.log(`Job: ${job.title} at ${job.company}`);
console.log(`Match Score: ${analysis.matchScore}%`);
console.log(`Recommendation: ${analysis.recommendation}`);
```

### 5. Generate Cover Letter

```javascript
// Generate personalized cover letter
const coverLetter = await agent.generateApplicationContent(
  job,
  myProfile,
  analysis
);

console.log('Cover Letter:');
console.log(coverLetter);
```

## Features

- ✅ **Intelligent Job Search** - AI-optimized search queries
- ✅ **Job Fit Analysis** - Match scoring with detailed insights
- ✅ **Cover Letter Generation** - Personalized application content
- ✅ **Batch Processing** - Analyze multiple jobs at once
- ✅ **Real-time Search** - Leverages Perplexity's online search
- ✅ **Customizable** - Configure all search and matching parameters

## Files

- `linkedin_agent.js` - Main agent class
- `linkedin_agent_config.js` - Configuration settings
- `linkedin_agent_example.js` - Usage examples

## Configuration

Edit `linkedin_agent_config.js` to customize:

```javascript
// API Settings
perplexity: {
  apiKey: process.env.PERPLEXITY_API_KEY || '', // Set via environment variable
  model: 'llama-3.1-sonar-large-128k-online'
}

// Application Settings
application: {
  autoApply: false,           // Manual review recommended
  maxApplicationsPerSession: 10,
  minMatchScore: 70          // Only apply to 70%+ matches
}
```

## Full Example: Complete Workflow

```javascript
// 1. Configure search
const searchParams = {
  keywords: 'Software Engineer',
  location: 'San Francisco, CA',
  jobType: 'full-time',
  remote: true
};

// 2. Define your profile
const userProfile = {
  name: 'Your Name',
  currentTitle: 'Senior Software Engineer',
  yearsOfExperience: 5,
  technicalSkills: ['JavaScript', 'React', 'Node.js', 'AWS'],
  targetRoles: ['Software Engineer', 'Tech Lead']
};

// 3. Run workflow
const results = await agent.runJobSearchWorkflow(searchParams, userProfile);

// 4. Review results
console.log(`Total jobs: ${results.totalJobs}`);
console.log(`Analyzed: ${results.analyzed}`);
console.log(`Ready to apply: ${results.readyToApply}`);

// 5. View top matches
results.results
  .filter(r => r.analysis.matchScore >= 70)
  .forEach(r => {
    console.log(`\n${r.job.title} at ${r.job.company}`);
    console.log(`Match: ${r.analysis.matchScore}%`);
    if (r.applicationContent) {
      console.log('Cover letter ready!');
    }
  });
```

## Examples

Run pre-built examples from the console:

```javascript
// Extract jobs from current LinkedIn page
exampleExtractCurrentPage()

// Analyze a specific job
exampleAnalyzeJob()

// Generate cover letter
exampleGenerateCoverLetter()

// Full search workflow
exampleBasicSearch()

// Batch analysis
exampleBatchAnalysis()
```

## API Reference

### Constructor

```javascript
new LinkedInAgent({
  apiKey: string,          // Perplexity API key
  model: string,           // AI model to use
  autoApply: boolean,      // Enable auto-application
  maxApplications: number  // Max applications per session
})
```

### Methods

| Method | Description | Returns |
|--------|-------------|---------|
| `initialize()` | Verify API connection | `Promise<boolean>` |
| `searchJobs(params)` | Search for jobs | `Promise<Job[]>` |
| `extractJobListings()` | Extract jobs from page | `Promise<Job[]>` |
| `analyzeJobFit(job, profile)` | Analyze job match | `Promise<Analysis>` |
| `generateApplicationContent(job, profile, analysis)` | Generate cover letter | `Promise<string>` |
| `runJobSearchWorkflow(params, profile)` | Complete workflow | `Promise<Results>` |

## Best Practices

1. **Rate Limiting**: Add delays between requests
   ```javascript
   await new Promise(r => setTimeout(r, 2000));
   ```

2. **Profile Accuracy**: Keep your profile detailed and current

3. **Review Content**: Always review generated cover letters before applying

4. **Specific Searches**: Use specific keywords for better results

5. **Monitor Usage**: Check API token consumption

## Troubleshooting

### "API key not found"
```javascript
// Update API key
setPerplexityApiKey('your-api-key');
```

### "No jobs found"
- Ensure you're on a LinkedIn jobs page
- Try a broader search query
- Check LinkedIn is fully loaded

### Rate limiting
```javascript
// Increase delays in config
advanced: {
  delayBetweenActions: 3000
}
```

## Privacy

- API key stored locally
- Profile data only used for matching
- No data transmitted except to Perplexity API
- You control all applications

## Documentation

For detailed documentation, see: `/docs/linkedin-agent-guide.md`

## Support

- Check examples in `linkedin_agent_example.js`
- Review full documentation
- File issues on BrowserOS GitHub

---

**Happy Job Hunting!** 🚀

Built with ❤️ using Perplexity AI and BrowserOS
