/**
 * LinkedIn Agent - Example Usage
 *
 * This file demonstrates how to use the LinkedIn automation agent
 * for job search and application.
 */

// Example 1: Basic Job Search
async function exampleBasicSearch() {
  // Initialize the agent with your API key
  const agent = new LinkedInAgent({
    apiKey: 'YOUR_PERPLEXITY_API_KEY', // Replace with your actual API key
    model: 'llama-3.1-sonar-large-128k-online',
    autoApply: false, // Don't auto-apply
    maxApplications: 10
  });

  // Define search parameters
  const searchParams = {
    keywords: 'Software Engineer',
    location: 'San Francisco, CA',
    jobType: 'full-time',
    experienceLevel: 'mid-senior',
    remote: true
  };

  // Define your profile
  const userProfile = {
    name: 'Your Name',
    currentTitle: 'Senior Software Engineer',
    yearsOfExperience: 5,
    targetRoles: ['Software Engineer', 'Backend Engineer', 'Full Stack Engineer'],
    technicalSkills: ['JavaScript', 'Python', 'React', 'Node.js', 'AWS'],
    experience: [
      {
        title: 'Senior Software Engineer',
        company: 'Tech Company',
        duration: '2020-Present',
        achievements: [
          'Led development of microservices architecture',
          'Reduced API response time by 40%',
          'Mentored 3 junior developers'
        ],
        technologies: ['Node.js', 'React', 'PostgreSQL', 'AWS']
      }
    ]
  };

  try {
    // Run the job search workflow
    const results = await agent.runJobSearchWorkflow(searchParams, userProfile);

    console.log('Job Search Results:');
    console.log(`- Total jobs found: ${results.totalJobs}`);
    console.log(`- Jobs analyzed: ${results.analyzed}`);
    console.log(`- Ready to apply: ${results.readyToApply}`);

    // Display results
    results.results.forEach((result, index) => {
      console.log(`\n${index + 1}. ${result.job.title} at ${result.job.company}`);
      console.log(`   Match Score: ${result.analysis.matchScore}%`);
      console.log(`   Recommendation: ${result.analysis.recommendation}`);
      if (result.analysis.reasons) {
        console.log(`   Reasons: ${result.analysis.reasons.join(', ')}`);
      }
    });

  } catch (error) {
    console.error('Error in job search:', error);
  }
}

// Example 2: Analyze a Specific Job
async function exampleAnalyzeJob() {
  const agent = new LinkedInAgent({
    apiKey: 'YOUR_PERPLEXITY_API_KEY' // Replace with your actual API key
  });

  await agent.initialize();

  const job = {
    title: 'Senior Backend Engineer',
    company: 'Amazing Tech Co',
    location: 'Remote',
    description: 'We are looking for a Senior Backend Engineer with 5+ years of experience in Node.js, PostgreSQL, and microservices architecture. You will lead the development of our core API services and mentor junior developers.'
  };

  const userProfile = {
    currentTitle: 'Senior Software Engineer',
    yearsOfExperience: 6,
    technicalSkills: ['Node.js', 'PostgreSQL', 'Microservices', 'AWS'],
    experience: [
      {
        title: 'Senior Software Engineer',
        achievements: ['Led microservices migration', 'Mentored team of 3']
      }
    ]
  };

  const analysis = await agent.analyzeJobFit(job, userProfile);
  console.log('Job Analysis:', analysis);
}

// Example 3: Generate Cover Letter
async function exampleGenerateCoverLetter() {
  const agent = new LinkedInAgent({
    apiKey: 'YOUR_PERPLEXITY_API_KEY' // Replace with your actual API key
  });

  await agent.initialize();

  const job = {
    title: 'Full Stack Engineer',
    company: 'Innovative Startup',
    description: 'Join our fast-growing team building the future of fintech. We need a full-stack engineer proficient in React, Node.js, and cloud technologies.'
  };

  const userProfile = {
    name: 'Your Name',
    currentTitle: 'Full Stack Developer',
    technicalSkills: ['React', 'Node.js', 'AWS', 'TypeScript'],
    yearsOfExperience: 4
  };

  const analysis = {
    matchScore: 85,
    talkingPoints: [
      'Strong React and Node.js experience',
      'Previous fintech experience',
      'Cloud infrastructure expertise'
    ]
  };

  const coverLetter = await agent.generateApplicationContent(job, userProfile, analysis);
  console.log('Generated Cover Letter:');
  console.log(coverLetter);
}

// Example 4: Using Configuration File
async function exampleWithConfig() {
  // Load configuration
  const config = getLinkedInAgentConfig();

  // Update user profile
  updateUserProfile({
    name: 'John Doe',
    email: 'john.doe@example.com',
    currentTitle: 'Senior Software Engineer',
    yearsOfExperience: 7,
    technicalSkills: ['JavaScript', 'Python', 'React', 'Django', 'AWS', 'Docker'],
    targetRoles: ['Engineering Manager', 'Senior Software Engineer', 'Tech Lead']
  });

  // Initialize agent with config
  const agent = new LinkedInAgent({
    apiKey: config.perplexity.apiKey,
    model: config.perplexity.model,
    autoApply: config.application.autoApply,
    maxApplications: config.application.maxApplicationsPerSession
  });

  // Run search with config defaults
  const searchParams = {
    keywords: 'Engineering Manager',
    location: config.search.defaultLocation,
    jobType: config.search.defaultJobType,
    remote: config.search.includeRemote
  };

  const results = await agent.runJobSearchWorkflow(searchParams, config.userProfile);
  return results;
}

// Example 5: Extract Jobs from Current LinkedIn Page
async function exampleExtractCurrentPage() {
  const agent = new LinkedInAgent({
    apiKey: 'YOUR_PERPLEXITY_API_KEY' // Replace with your actual API key
  });

  await agent.initialize();

  // Extract jobs from the page you're currently viewing
  console.log('Extracting jobs from current page...');
  const jobs = await agent.extractJobListings();

  console.log(`Found ${jobs.length} jobs on this page:`);
  jobs.forEach((job, index) => {
    console.log(`\n${index + 1}. ${job.title}`);
    console.log(`   Company: ${job.company}`);
    console.log(`   Location: ${job.location}`);
    console.log(`   Posted: ${job.postedDate}`);
    console.log(`   Link: ${job.link}`);
  });

  return jobs;
}

// Example 6: Batch Analysis of Jobs
async function exampleBatchAnalysis() {
  const agent = new LinkedInAgent({
    apiKey: 'YOUR_PERPLEXITY_API_KEY' // Replace with your actual API key
  });

  await agent.initialize();

  // Get jobs from current page
  const jobs = await agent.extractJobListings();

  const userProfile = {
    currentTitle: 'Software Engineer',
    yearsOfExperience: 3,
    technicalSkills: ['JavaScript', 'React', 'Node.js', 'Python'],
    targetRoles: ['Software Engineer', 'Full Stack Developer']
  };

  console.log(`Analyzing ${jobs.length} jobs...`);

  const analyses = [];
  for (const job of jobs.slice(0, 5)) { // Limit to first 5 for demo
    console.log(`\nAnalyzing: ${job.title} at ${job.company}`);

    const analysis = await agent.analyzeJobFit(job, userProfile);
    analyses.push({
      job,
      analysis
    });

    console.log(`Match Score: ${analysis.matchScore}%`);
    console.log(`Recommendation: ${analysis.recommendation}`);

    // Rate limiting
    await new Promise(resolve => setTimeout(resolve, 1500));
  }

  // Sort by match score
  analyses.sort((a, b) => b.analysis.matchScore - a.analysis.matchScore);

  console.log('\n\n=== Top Matches ===');
  analyses.slice(0, 3).forEach((item, index) => {
    console.log(`\n${index + 1}. ${item.job.title} at ${item.job.company}`);
    console.log(`   Score: ${item.analysis.matchScore}%`);
    console.log(`   ${item.analysis.recommendation}`);
  });

  return analyses;
}

// Console helper functions for browser DevTools
console.log(`
=== LinkedIn Agent Examples ===

To run these examples in the browser console:

1. Basic Search:
   exampleBasicSearch()

2. Analyze a Specific Job:
   exampleAnalyzeJob()

3. Generate Cover Letter:
   exampleGenerateCoverLetter()

4. Use Configuration:
   exampleWithConfig()

5. Extract Jobs from Current Page:
   exampleExtractCurrentPage()

6. Batch Analysis:
   exampleBatchAnalysis()

Quick Start (when on LinkedIn jobs page):
-----------------------------------------
const agent = new LinkedInAgent({
  apiKey: 'YOUR_PERPLEXITY_API_KEY'
});

await agent.initialize();
const jobs = await agent.extractJobListings();
console.log('Found jobs:', jobs);
`);

// Make examples available globally
if (typeof window !== 'undefined') {
  window.exampleBasicSearch = exampleBasicSearch;
  window.exampleAnalyzeJob = exampleAnalyzeJob;
  window.exampleGenerateCoverLetter = exampleGenerateCoverLetter;
  window.exampleWithConfig = exampleWithConfig;
  window.exampleExtractCurrentPage = exampleExtractCurrentPage;
  window.exampleBatchAnalysis = exampleBatchAnalysis;
}
