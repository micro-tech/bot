# Project-Specific Markmap Rules Override

This file overrides the global `markmap-docs.md` rules for this project only.

## Project Notes
- The main architecture mindmaps live in `.doc/markmap/`
- Old mindmaps in `.mindmap/` are being migrated
- Use the numbered `00-overview.mmd` → `11-utils-display.mmd` naming

## Additional Diagrams
- Architecture flowcharts are kept in `docs/diagram.mmd` and `docs/ARCHITECTURE.mmd` (pure Mermaid)
- These are separate from the Markmap documentation system

## Build Command
```powershell
.\scripts\build-markmaps.ps1
```