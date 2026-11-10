package pipeline

import (
	"context"
	"testing"
)

func TestSetupRunsDevSetupOnly(t *testing.T) {
	root := t.TempDir()
	r := &FakeRunner{}
	if err := Setup(context.Background(), root, r); err != nil {
		t.Fatal(err)
	}
	if len(r.Commands) != 1 || r.Commands[0] != "./tools/dev/setup.sh" {
		t.Fatalf("commands got %#v", r.Commands)
	}
}
