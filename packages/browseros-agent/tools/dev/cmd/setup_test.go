package cmd

import (
	"os"
	"path/filepath"
	"testing"
)

func TestBuildSetupPlanAlwaysInstallsDependencies(t *testing.T) {
	root := t.TempDir()

	plan := buildSetupPlan(root, true)

	if !plan.RunInstall {
		t.Fatal("expected dependency install to always run")
	}
}

func TestBuildSetupPlanIfNeededSkipsExistingGeneratedGraphQL(t *testing.T) {
	root := t.TempDir()
	writeGeneratedGraphQLSentinels(t, root)

	plan := buildSetupPlan(root, true)

	if plan.RunCodegen {
		t.Fatal("expected --if-needed setup to skip codegen when generated GraphQL exists")
	}
}

func TestBuildSetupPlanIfNeededRunsCodegenWhenGeneratedGraphQLEmpty(t *testing.T) {
	root := t.TempDir()
	generatedDir := filepath.Join(root, "apps/app/generated/graphql")
	if err := os.MkdirAll(generatedDir, 0o755); err != nil {
		t.Fatal(err)
	}

	plan := buildSetupPlan(root, true)

	if !plan.RunCodegen {
		t.Fatal("expected --if-needed setup to run codegen when generated GraphQL is empty")
	}
}

func TestBuildSetupPlanIfNeededRunsCodegenWhenGeneratedGraphQLMissing(t *testing.T) {
	root := t.TempDir()

	plan := buildSetupPlan(root, true)

	if !plan.RunCodegen {
		t.Fatal("expected --if-needed setup to run codegen when generated GraphQL is missing")
	}
}

func TestBuildSetupPlanExplicitSetupRunsCodegen(t *testing.T) {
	root := t.TempDir()
	writeGeneratedGraphQLSentinels(t, root)

	plan := buildSetupPlan(root, false)

	if !plan.RunCodegen {
		t.Fatal("expected explicit setup to refresh codegen")
	}
}

func writeGeneratedGraphQLSentinels(t *testing.T, root string) {
	t.Helper()
	generatedDir := filepath.Join(root, "apps/app/generated/graphql")
	if err := os.MkdirAll(generatedDir, 0o755); err != nil {
		t.Fatal(err)
	}
	for _, file := range []string{"gql.ts", "graphql.ts", "schema.graphql"} {
		if err := os.WriteFile(filepath.Join(generatedDir, file), []byte("generated"), 0o644); err != nil {
			t.Fatal(err)
		}
	}
}
