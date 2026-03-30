package cmd

import (
	"testing"
	"time"
)

func TestCommandName(t *testing.T) {
	tests := []struct {
		name string
		args []string
		want string
	}{
		{"empty args", nil, "unknown"},
		{"known command", []string{"health"}, "browseros-cli health"},
		{"unknown command", []string{"nonexistent"}, "unknown"},
		{"subcommand", []string{"bookmark", "search"}, "browseros-cli bookmark search"},
		{"known with extra args", []string{"snap", "--enhanced"}, "browseros-cli snap"},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := commandName(tt.args)
			if got != tt.want {
				t.Errorf("commandName(%v) = %q, want %q", tt.args, got, tt.want)
			}
		})
	}
}

func TestPrimaryCommand(t *testing.T) {
	tests := []struct {
		name string
		args []string
		want string
	}{
		{"empty", nil, ""},
		{"root flag then command", []string{"--json", "update"}, "update"},
		{"subcommand", []string{"bookmark", "update"}, "bookmark"},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := primaryCommand(tt.args); got != tt.want {
				t.Fatalf("primaryCommand(%v) = %q, want %q", tt.args, got, tt.want)
			}
		})
	}
}

func TestRequestedBoolFlag(t *testing.T) {
	if !requestedBoolFlag([]string{"--json"}, "--json", false) {
		t.Fatal("requestedBoolFlag() = false, want true")
	}
	if !requestedBoolFlag([]string{"--debug=true"}, "--debug", false) {
		t.Fatal("requestedBoolFlag() with assignment = false, want true")
	}
	if requestedBoolFlag([]string{"--debug=false"}, "--debug", false) {
		t.Fatal("requestedBoolFlag() with false assignment = true, want false")
	}
}

func TestShouldSkipAutomaticUpdates(t *testing.T) {
	tests := []struct {
		name string
		args []string
		want bool
	}{
		{"short help flag", []string{"-h"}, true},
		{"help flag", []string{"--help"}, true},
		{"version flag", []string{"--version"}, true},
		{"update command", []string{"update"}, true},
		{"bookmark update subcommand", []string{"bookmark", "update"}, false},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			if got := shouldSkipAutomaticUpdates(tt.args); got != tt.want {
				t.Fatalf("shouldSkipAutomaticUpdates(%v) = %t, want %t", tt.args, got, tt.want)
			}
		})
	}
}

func TestDrainAutomaticUpdateCheckWithTimeoutWaitsForCompletion(t *testing.T) {
	done := make(chan struct{})
	returned := make(chan struct{})

	go func() {
		drainAutomaticUpdateCheckWithTimeout(done, time.Second)
		close(returned)
	}()

	select {
	case <-returned:
		t.Fatal("drainAutomaticUpdateCheckWithTimeout() returned before check completed")
	case <-time.After(10 * time.Millisecond):
	}

	close(done)

	select {
	case <-returned:
	case <-time.After(100 * time.Millisecond):
		t.Fatal("drainAutomaticUpdateCheckWithTimeout() did not return after check completed")
	}
}

func TestDrainAutomaticUpdateCheckWithTimeoutStopsWaiting(t *testing.T) {
	done := make(chan struct{})
	returned := make(chan struct{})

	go func() {
		drainAutomaticUpdateCheckWithTimeout(done, 20*time.Millisecond)
		close(returned)
	}()

	select {
	case <-returned:
		t.Fatal("drainAutomaticUpdateCheckWithTimeout() returned before timeout elapsed")
	case <-time.After(5 * time.Millisecond):
	}

	select {
	case <-returned:
	case <-time.After(100 * time.Millisecond):
		t.Fatal("drainAutomaticUpdateCheckWithTimeout() did not return after timeout")
	}
}
