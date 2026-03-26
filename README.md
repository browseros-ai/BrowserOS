# Shimmy-Browser

Shimmy-Browser is a fork of BrowserOS focused on custom runtime behavior, agent UX improvements, and Sup-agent integration via submodule.

- Browser fork repo: `https://github.com/Karthikprasadm/Shimmy-Browser.git`
- Upstream browser repo: `https://github.com/browseros-ai/BrowserOS.git`
- Sup-agent repo: `https://github.com/Karthikprasadm/Sup-agent.git`

## Project Structure

- `packages/browseros-agent`: Agent/server monorepo used for BrowserOS AI runtime.
- `packages/browseros-agent/vendor/sup-agent`: Sup-agent Git submodule.
- `docs/`: Documentation and contributor guides.

## Sup-agent Submodule

Sup-agent is managed as a Git submodule at:

`packages/browseros-agent/vendor/sup-agent`

Please follow:

- `docs/submodule-workflow.md`

for sync, commit, and update workflow between parent and submodule.

## Local Development (Agent/Server)

After cloning, initialize the Sup-agent submodule (required for built-in skill defaults and server typecheck):

```bash
git submodule update --init --recursive
```

From repo root:

```bash
cd packages/browseros-agent
bun install
```

If `bun install` or the server build fails with missing `vendor/sup-agent/...` paths, run the submodule command above and retry.

Build everything:

```bash
bun run build
```

Run runtime:

```bash
bun run start:agent
bun run start:server
```

## Contributing

- Main guide: `CONTRIBUTING.md`
- Submodule workflow: `docs/submodule-workflow.md`

## License

This project follows the same licensing model as upstream BrowserOS (AGPL-3.0). See `LICENSE`.

## Citation

If you use BrowserOS in your research or project, please cite:

```bibtex
@software{browseros2025,
  author = {Sonti, Nithin and Sonti, Nikhil and {BrowserOS-team}},
  title = {BrowserOS: The open-source Agentic browser},
  url = {https://github.com/browseros-ai/BrowserOS},
  year = {2025},
  publisher = {GitHub},
  license = {AGPL-3.0},
}
```
