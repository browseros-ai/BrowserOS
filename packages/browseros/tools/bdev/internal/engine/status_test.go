package engine

import "testing"

func TestStatusReflectsAheadBehindAndSynced(t *testing.T) {
	t.Parallel()

	ctx := setupPullFixture(t)

	before, err := Status(ctx, true)
	if err != nil {
		t.Fatalf("Status before pull: %v", err)
	}
	if before.Ahead != 1 || before.Behind != 1 || before.Synced != 0 {
		t.Fatalf("unexpected status before pull: ahead=%d behind=%d synced=%d", before.Ahead, before.Behind, before.Synced)
	}
	if !contains(before.AheadFiles, "orphan.txt") {
		t.Fatalf("expected orphan.txt in ahead files, got %#v", before.AheadFiles)
	}
	if !contains(before.BehindFiles, "foo.txt") {
		t.Fatalf("expected foo.txt in behind files, got %#v", before.BehindFiles)
	}
}
