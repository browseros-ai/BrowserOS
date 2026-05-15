import { useState, useEffect, useCallback } from 'react';

interface AccessibilityProfile {
  id: string;
  name: string;
  icon: string;
  settings: {
    visual: any;
    audio: any;
    haptic?: any;
    cognitive: any;
  };
}

export const useAccessibility = () => {
  const [currentProfile, setCurrentProfile] = useState<AccessibilityProfile | null>(null);
  const [availableProfiles, setAvailableProfiles] = useState<AccessibilityProfile[]>([]);
  const [isAutoDetecting, setIsAutoDetecting] = useState(true);

  useEffect(() => {
    const loadProfiles = async () => {
      try {
        const response = await fetch('/config/sensory-profiles.json');
        const data = await response.json();
        const profiles = data.profiles || [];
        setAvailableProfiles(profiles);
        
        if (profiles.length > 0) {
          setCurrentProfile(profiles[0]);
          setIsAutoDetecting(false);
        }
      } catch (error) {
        console.error('無法載入無障礙配置:', error);
      }
    };
    loadProfiles();
  }, []);

  const applyProfile = useCallback((profileId: string) => {
    const profile = availableProfiles.find(p => p.id === profileId);
    if (profile) {
      setCurrentProfile(profile);
      localStorage.setItem('accessibility_profile', profileId);
      setIsAutoDetecting(false);
    }
  }, [availableProfiles]);

  const adjustSettings = useCallback((settings: any) => {
    if (currentProfile) {
      const updated = { ...currentProfile, settings: { ...currentProfile.settings, ...settings } };
      setCurrentProfile(updated);
    }
  }, [currentProfile]);

  return { currentProfile, availableProfiles, isAutoDetecting, applyProfile, adjustSettings };
};
