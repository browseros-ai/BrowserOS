/**
 * LinkedIn Job Automation Agent using Perplexity AI
 *
 * This agent automates LinkedIn job search and application processes
 * using Perplexity AI for intelligent decision making and content generation.
 */

class LinkedInAgent {
  constructor(config = {}) {
    this.apiKey = config.apiKey || '';
    this.baseUrl = 'https://api.perplexity.ai';
    this.model = config.model || 'llama-3.1-sonar-large-128k-online';
    this.searchFilters = config.searchFilters || {};
    this.autoApply = config.autoApply || false;
    this.maxApplications = config.maxApplications || 10;
    this.applicationsSubmitted = 0;
  }

  /**
   * Initialize the agent and verify API connection
   */
  async initialize() {
    if (!this.apiKey) {
      throw new Error('Perplexity API key is required');
    }

    console.log('LinkedInAgent: Initializing...');

    // Verify API connection
    try {
      await this.testConnection();
      console.log('LinkedInAgent: Successfully connected to Perplexity API');
      return true;
    } catch (error) {
      console.error('LinkedInAgent: Failed to connect to Perplexity API:', error);
      throw error;
    }
  }

  /**
   * Test connection to Perplexity API
   */
  async testConnection() {
    const response = await fetch(`${this.baseUrl}/chat/completions`, {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${this.apiKey}`,
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        model: this.model,
        messages: [
          {
            role: 'user',
            content: 'Hello, this is a connection test.'
          }
        ],
        max_tokens: 10
      })
    });

    if (!response.ok) {
      const error = await response.json();
      throw new Error(`API Error: ${error.message || response.statusText}`);
    }

    return await response.json();
  }

  /**
   * Query Perplexity AI with a prompt
   */
  async queryPerplexity(prompt, options = {}) {
    const response = await fetch(`${this.baseUrl}/chat/completions`, {
      method: 'POST',
      headers: {
        'Authorization': `Bearer ${this.apiKey}`,
        'Content-Type': 'application/json'
      },
      body: JSON.stringify({
        model: this.model,
        messages: [
          {
            role: 'system',
            content: options.systemPrompt || 'You are a helpful AI assistant specializing in job search and career guidance.'
          },
          {
            role: 'user',
            content: prompt
          }
        ],
        max_tokens: options.maxTokens || 2000,
        temperature: options.temperature || 0.2,
        top_p: options.topP || 0.9,
        return_citations: options.returnCitations !== false,
        search_domain_filter: options.searchDomainFilter || [],
        return_images: false,
        return_related_questions: options.returnRelatedQuestions || false,
        search_recency_filter: options.searchRecencyFilter || 'month',
        stream: false
      })
    });

    if (!response.ok) {
      const error = await response.json();
      throw new Error(`Perplexity API Error: ${error.message || response.statusText}`);
    }

    const data = await response.json();
    return {
      content: data.choices[0].message.content,
      citations: data.citations || [],
      usage: data.usage
    };
  }

  /**
   * Navigate to LinkedIn jobs page
   */
  async navigateToLinkedInJobs() {
    console.log('LinkedInAgent: Navigating to LinkedIn Jobs...');

    // Check if already on LinkedIn
    const currentUrl = window.location.href;
    if (!currentUrl.includes('linkedin.com')) {
      window.location.href = 'https://www.linkedin.com/jobs/';
      return new Promise(resolve => setTimeout(resolve, 3000));
    }

    // Navigate to jobs section if not already there
    if (!currentUrl.includes('/jobs')) {
      window.location.href = 'https://www.linkedin.com/jobs/';
      return new Promise(resolve => setTimeout(resolve, 3000));
    }
  }

  /**
   * Search for jobs based on filters
   */
  async searchJobs(searchParams) {
    console.log('LinkedInAgent: Searching for jobs...', searchParams);

    const {
      keywords = '',
      location = '',
      jobType = '', // full-time, part-time, contract, etc.
      experienceLevel = '', // entry, associate, mid-senior, director, executive
      remote = false
    } = searchParams;

    // Use Perplexity to refine search query
    const searchOptimization = await this.queryPerplexity(
      `I'm searching for jobs on LinkedIn with these criteria:
      - Keywords: ${keywords}
      - Location: ${location}
      - Job Type: ${jobType}
      - Experience Level: ${experienceLevel}
      - Remote: ${remote}

      Suggest the most effective search keywords and filters to find relevant job postings.`,
      {
        systemPrompt: 'You are a LinkedIn job search expert. Provide concise, actionable search strategies.',
        maxTokens: 500
      }
    );

    console.log('LinkedInAgent: Search optimization suggestions:', searchOptimization.content);

    // Build LinkedIn search URL
    const searchUrl = this.buildLinkedInSearchUrl({
      keywords,
      location,
      jobType,
      experienceLevel,
      remote
    });

    // Navigate to search results
    window.location.href = searchUrl;
    await new Promise(resolve => setTimeout(resolve, 3000));

    return this.extractJobListings();
  }

  /**
   * Build LinkedIn search URL with filters
   */
  buildLinkedInSearchUrl(params) {
    const baseUrl = 'https://www.linkedin.com/jobs/search/';
    const urlParams = new URLSearchParams();

    if (params.keywords) urlParams.append('keywords', params.keywords);
    if (params.location) urlParams.append('location', params.location);
    if (params.jobType) urlParams.append('f_JT', params.jobType);
    if (params.experienceLevel) urlParams.append('f_E', params.experienceLevel);
    if (params.remote) urlParams.append('f_WT', '2'); // 2 = Remote

    return `${baseUrl}?${urlParams.toString()}`;
  }

  /**
   * Extract job listings from the current page
   */
  async extractJobListings() {
    console.log('LinkedInAgent: Extracting job listings...');

    // Wait for page to load
    await new Promise(resolve => setTimeout(resolve, 2000));

    const jobs = [];

    // Try to find job cards (LinkedIn's structure may vary)
    const jobCards = document.querySelectorAll('.job-card-container, .jobs-search-results__list-item, [data-job-id]');

    console.log(`LinkedInAgent: Found ${jobCards.length} job cards`);

    for (const card of Array.from(jobCards).slice(0, 20)) {
      try {
        const job = {
          id: card.getAttribute('data-job-id') || card.id || '',
          title: card.querySelector('.job-card-list__title, .artdeco-entity-lockup__title')?.textContent?.trim() || '',
          company: card.querySelector('.job-card-container__company-name, .artdeco-entity-lockup__subtitle')?.textContent?.trim() || '',
          location: card.querySelector('.job-card-container__metadata-item, .artdeco-entity-lockup__caption')?.textContent?.trim() || '',
          description: card.querySelector('.job-card-list__snippet')?.textContent?.trim() || '',
          link: card.querySelector('a')?.href || '',
          postedDate: card.querySelector('.job-card-container__listed-time, time')?.textContent?.trim() || ''
        };

        if (job.title && job.company) {
          jobs.push(job);
        }
      } catch (error) {
        console.error('LinkedInAgent: Error extracting job card:', error);
      }
    }

    console.log(`LinkedInAgent: Extracted ${jobs.length} job listings`);
    return jobs;
  }

  /**
   * Analyze a job posting to determine fit
   */
  async analyzeJobFit(job, userProfile) {
    console.log('LinkedInAgent: Analyzing job fit...', job.title);

    const prompt = `Analyze this job posting for fit:

Job Title: ${job.title}
Company: ${job.company}
Location: ${job.location}
Description: ${job.description}

User Profile:
${JSON.stringify(userProfile, null, 2)}

Provide:
1. Match score (0-100)
2. Key reasons for the score
3. Recommendation (Apply, Maybe, Skip)
4. Tailored talking points for application

Format your response as JSON.`;

    const analysis = await this.queryPerplexity(prompt, {
      systemPrompt: 'You are a career advisor AI. Analyze job fit objectively and provide actionable insights. Always respond in valid JSON format.',
      maxTokens: 1000,
      temperature: 0.1
    });

    try {
      // Extract JSON from response
      const jsonMatch = analysis.content.match(/\{[\s\S]*\}/);
      if (jsonMatch) {
        return JSON.parse(jsonMatch[0]);
      }

      // Fallback if no JSON found
      return {
        matchScore: 50,
        recommendation: 'Maybe',
        reasons: [analysis.content],
        talkingPoints: []
      };
    } catch (error) {
      console.error('LinkedInAgent: Error parsing job analysis:', error);
      return {
        matchScore: 0,
        recommendation: 'Skip',
        reasons: ['Error analyzing job'],
        talkingPoints: []
      };
    }
  }

  /**
   * Generate a tailored cover letter or application message
   */
  async generateApplicationContent(job, userProfile, analysis) {
    console.log('LinkedInAgent: Generating application content...');

    const prompt = `Generate a compelling, personalized cover letter for this job application:

Job Title: ${job.title}
Company: ${job.company}
Job Description: ${job.description}

User Profile:
${JSON.stringify(userProfile, null, 2)}

Job Fit Analysis:
Match Score: ${analysis.matchScore}
Talking Points: ${analysis.talkingPoints?.join(', ')}

Requirements:
- Professional and engaging tone
- Highlight relevant experience and skills
- Maximum 250 words
- Include specific examples where possible
- Show genuine interest in the role and company`;

    const result = await this.queryPerplexity(prompt, {
      systemPrompt: 'You are an expert resume writer and career coach. Create compelling, authentic application content.',
      maxTokens: 800,
      temperature: 0.3
    });

    return result.content;
  }

  /**
   * Apply to a job (requires user interaction for now)
   */
  async applyToJob(job, applicationContent) {
    console.log('LinkedInAgent: Preparing to apply to job...', job.title);

    // Navigate to job posting
    if (job.link) {
      window.open(job.link, '_blank');
      await new Promise(resolve => setTimeout(resolve, 2000));
    }

    // Return application content for user to submit
    return {
      job,
      coverLetter: applicationContent,
      status: 'ready_to_apply',
      instructions: 'Please review and submit the application manually. Auto-submission requires additional permissions.'
    };
  }

  /**
   * Main workflow: Find and apply to jobs
   */
  async runJobSearchWorkflow(searchParams, userProfile) {
    console.log('LinkedInAgent: Starting job search workflow...');

    try {
      // Initialize
      await this.initialize();

      // Navigate to LinkedIn
      await this.navigateToLinkedInJobs();

      // Search for jobs
      const jobs = await this.searchJobs(searchParams);
      console.log(`LinkedInAgent: Found ${jobs.length} jobs`);

      const results = [];
      const applicationsToSubmit = [];

      // Analyze each job
      for (const job of jobs) {
        if (this.applicationsSubmitted >= this.maxApplications) {
          console.log('LinkedInAgent: Reached maximum applications limit');
          break;
        }

        // Analyze job fit
        const analysis = await this.analyzeJobFit(job, userProfile);

        console.log(`LinkedInAgent: Job "${job.title}" - Match: ${analysis.matchScore}%, Recommendation: ${analysis.recommendation}`);

        const result = {
          job,
          analysis,
          status: 'analyzed'
        };

        // If good fit and auto-apply is enabled
        if (analysis.matchScore >= 70 && this.autoApply) {
          const applicationContent = await this.generateApplicationContent(job, userProfile, analysis);
          result.applicationContent = applicationContent;
          result.status = 'ready_to_apply';
          applicationsToSubmit.push(result);
          this.applicationsSubmitted++;
        }

        results.push(result);

        // Rate limiting - wait between API calls
        await new Promise(resolve => setTimeout(resolve, 1000));
      }

      return {
        totalJobs: jobs.length,
        analyzed: results.length,
        readyToApply: applicationsToSubmit.length,
        results,
        applicationsToSubmit
      };

    } catch (error) {
      console.error('LinkedInAgent: Workflow error:', error);
      throw error;
    }
  }

  /**
   * Get current agent status
   */
  getStatus() {
    return {
      initialized: !!this.apiKey,
      applicationsSubmitted: this.applicationsSubmitted,
      maxApplications: this.maxApplications,
      autoApply: this.autoApply,
      model: this.model
    };
  }
}

// Export for use in other modules
if (typeof module !== 'undefined' && module.exports) {
  module.exports = LinkedInAgent;
}

// Make available globally in browser context
if (typeof window !== 'undefined') {
  window.LinkedInAgent = LinkedInAgent;
}
