package profile

import (
	"os"
	"path/filepath"
	"testing"
)

func TestReadProfilesFromLocalState(t *testing.T) {
	dir := t.TempDir()
	localState := `{
	  "profile": {
	    "info_cache": {
	      "Default": {"name": "Personal", "user_name": "me@example.com"},
	      "Profile 25": {"name": "Work", "user_name": "work@example.com"}
	    }
	  }
	}`
	if err := os.WriteFile(filepath.Join(dir, "Local State"), []byte(localState), 0644); err != nil {
		t.Fatal(err)
	}
	profiles, err := ReadProfiles(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(profiles) != 2 {
		t.Fatalf("expected 2 profiles, got %d", len(profiles))
	}
	if profiles[0].Dir != "Default" || profiles[1].Dir != "Profile 25" {
		t.Fatalf("profiles not sorted by dir: %+v", profiles)
	}
	if profiles[1].Name != "Work" || profiles[1].Email != "work@example.com" {
		t.Fatalf("profile metadata mismatch: %+v", profiles[1])
	}
}

func TestReadProfilesFallbackDefault(t *testing.T) {
	dir := t.TempDir()
	profiles, err := ReadProfiles(dir)
	if err != nil {
		t.Fatal(err)
	}
	if len(profiles) != 1 || profiles[0].Dir != "Default" {
		t.Fatalf("fallback mismatch: %+v", profiles)
	}
}
