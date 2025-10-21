/**
 * LinkedIn Agent Configuration
 *
 * This file contains configuration for the LinkedIn automation agent
 * using Perplexity AI.
 */

const LinkedInAgentConfig = {
  // Perplexity API Configuration
  perplexity: {
    apiKey: process.env.PERPLEXITY_API_KEY || '', // Set your API key here or use environment variable
    baseUrl: 'https://api.perplexity.ai',
    model: 'llama-3.1-sonar-large-128k-online', // Best for online search and job research
    // Alternative models:
    // - 'llama-3.1-sonar-small-128k-online' (faster, lower cost)
    // - 'llama-3.1-sonar-huge-128k-online' (most powerful)
    // - 'llama-3.1-sonar-large-128k-chat' (offline mode, no web search)
  },

  // Job Search Defaults
  search: {
    maxResults: 20,
    defaultLocation: 'United States',
    defaultJobType: 'full-time',
    defaultExperienceLevel: 'mid-senior',
    includeRemote: true,
    searchRecency: 'week', // 'day', 'week', 'month'
  },

  // Application Settings
  application: {
    autoApply: false, // Set to true to enable automatic applications (use with caution!)
    maxApplicationsPerSession: 10,
    minMatchScore: 70, // Only apply to jobs with 70%+ match score
    reviewBeforeApply: true, // Show generated content before applying
  },

  // User Profile Template
  // Customize this with your information
  userProfile: {
    name: '',
    email: '',
    phone: '',
    location: '',

    // Professional Information
    currentTitle: '',
    yearsOfExperience: 0,
    targetRoles: [],
    targetIndustries: [],

    // Skills
    technicalSkills: [],
    softSkills: [],
    certifications: [],

    // Education
    education: [
      {
        degree: '',
        field: '',
        institution: '',
        year: ''
      }
    ],

    // Work Experience (most recent 3-5 positions)
    experience: [
      {
        title: '',
        company: '',
        duration: '',
        achievements: [],
        technologies: []
      }
    ],

    // Preferences
    preferences: {
      minSalary: 0,
      maxCommute: '',
      workAuthorization: '',
      willingToRelocate: false,
      preferredCompanySize: '', // startup, small, medium, large, enterprise
      preferredCompanyCulture: [],
    },

    // Portfolio & Links
    links: {
      linkedin: '',
      github: '',
      portfolio: '',
      website: ''
    }
  },

  // Advanced Settings
  advanced: {
    // Rate limiting (to avoid triggering LinkedIn's anti-bot measures)
    delayBetweenActions: 2000, // milliseconds
    delayBetweenApplications: 5000,
    maxScrollAttempts: 3,

    // Content Generation
    coverLetterMaxWords: 250,
    coverLetterTone: 'professional', // 'professional', 'friendly', 'enthusiastic'

    // Logging
    enableLogging: true,
    logLevel: 'info', // 'debug', 'info', 'warn', 'error'
  }
};

// Helper function to get config
function getLinkedInAgentConfig() {
  return LinkedInAgentConfig;
}

// Helper function to update API key
function setPerplexityApiKey(apiKey) {
  LinkedInAgentConfig.perplexity.apiKey = apiKey;
}

// Helper function to update user profile
function updateUserProfile(profileData) {
  LinkedInAgentConfig.userProfile = {
    ...LinkedInAgentConfig.userProfile,
    ...profileData
  };
}

// Export for use in other modules
if (typeof module !== 'undefined' && module.exports) {
  module.exports = {
    LinkedInAgentConfig,
    getLinkedInAgentConfig,
    setPerplexityApiKey,
    updateUserProfile
  };
}

// Make available globally in browser context
if (typeof window !== 'undefined') {
  window.LinkedInAgentConfig = LinkedInAgentConfig;
  window.getLinkedInAgentConfig = getLinkedInAgentConfig;
  window.setPerplexityApiKey = setPerplexityApiKey;
  window.updateUserProfile = updateUserProfile;
}
