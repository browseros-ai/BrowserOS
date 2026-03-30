package app

import (
	"fmt"
	"os"
	"path/filepath"

	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/repo"
	"github.com/browseros-ai/BrowserOS/packages/browseros/tools/bdev/internal/workspace"
)

type App struct {
	JSON     bool
	Verbose  bool
	CWD      string
	Config   *workspace.Config
	Registry *workspace.Registry
}

func Load(jsonOut bool, verbose bool, cwd string) (*App, error) {
	if cwd == "" {
		var err error
		cwd, err = os.Getwd()
		if err != nil {
			return nil, err
		}
	}
	cfg, err := workspace.LoadConfig()
	if err != nil {
		return nil, err
	}
	reg, err := workspace.LoadRegistry()
	if err != nil {
		return nil, err
	}
	return &App{
		JSON:     jsonOut,
		Verbose:  verbose,
		CWD:      filepath.Clean(cwd),
		Config:   cfg,
		Registry: reg,
	}, nil
}

func (a *App) Save() error {
	if err := workspace.SaveConfig(a.Config); err != nil {
		return err
	}
	return workspace.SaveRegistry(a.Registry)
}

func (a *App) ResolveWorkspace(name string, src string) (workspace.Entry, error) {
	return workspace.Resolve(a.Registry, name, a.CWD, src)
}

func (a *App) RepoInfo() (*repo.Info, error) {
	if a.Config.PatchesRepo == "" {
		discovered, err := repo.Discover(a.CWD)
		if err != nil {
			return nil, fmt.Errorf(
				`patches repo is not configured; run "bdev add <name> <path> --patches-repo <repo>" from the browseros repo once`,
			)
		}
		return repo.Load(discovered)
	}
	return repo.Load(a.Config.PatchesRepo)
}
