# Frontend React + TypeScript Rewrite

## Goal

Convert the `web` frontend from vanilla JavaScript classes to TypeScript 7.0.2 + React 19 + react-leaflet, preserving the existing UI/UX and behavior.

## Decision Record

| Choice | Decision |
|--------|----------|
| Language | TypeScript 7.0.2 (latest stable at time of writing) |
| UI framework | React 19 |
| Leaflet binding | `react-leaflet` v5 (latest stable) |
| State management | React Context + hooks |
| UI redesign | No — keep current look and behavior |
| Build tool | Vite with `@vitejs/plugin-react` |
| Package manager | `bun` (existing) |

## Current State

The frontend in `web/src/js` is a class-based vanilla JS application:

- `Squaremap.js` — global `S` singleton, creates the Leaflet map, loads `tiles/settings.json`, and bootstraps all other modules.
- `LayerControl.js` — Leaflet layer control, tile layer swapping, overlay visibility persistence.
- `WorldList.js` / `World.js` — world list, world switching, per-world settings, marker layers.
- `PlayerList.js` / `Player.js` — player list, head icons, nameplates, marker rotation, following.
- `Sidebar.js` / `Pin.js` / `Fieldset.js` — DOM sidebar and controls.
- `UICoordinates.js` / `UILink.js` — custom Leaflet controls.
- `SquaremapTileLayer.js` — custom `L.TileLayer` that fetches tiles as `blob()` and revokes object URLs.
- `Markers.js` — marker factories for rectangle, polyline, polygon, circle, ellipse, icon.
- `addons/Ellipse.js` / `addons/RotateMarker.js` — Leaflet prototype extensions.
- `types.ts` — existing TypeScript interfaces for settings/world/player data.

## Target Architecture

### Entry point

- `index.html` loads `src/main.tsx` instead of `src/js/Squaremap.js`.
- `main.tsx` renders `<App />` into a React root.
- `App.tsx` fetches `tiles/settings.json` and mounts the map once loaded.

### State

A single `MapContext` (React Context) holds:

- `settings` — global settings from `tiles/settings.json`.
- `worlds` — `Map<string, World>`.
- `currentWorld` — currently active `World`.
- `players` — `PlayerData[]` and poll state.
- `markerLayers` — per-world marker layer state.
- `mapRef` — ref to the Leaflet `Map` instance from `react-leaflet`.
- `showSidebar`, `showCoordinates`, `showLink`, `showControls` — URL-derived UI toggles.

Context consumers:

- `MapView` subscribes to `currentWorld` and reconfigures the map.
- `Sidebar` subscribes to `worlds` and `players`.
- `PlayerList` subscribes to `players`.
- `WorldList` subscribes to `worlds` and `currentWorld`.

### Components

| Component | Responsibility |
|-----------|----------------|
| `App` | Fetch settings, render loading/error/success states. |
| `MapView` | `react-leaflet` `MapContainer` with CRS.Simple, custom controls, tile layers, and player markers. |
| `SquaremapTileLayer` | `react-leaflet` custom `TileLayer` preserving `fetch(blob())` + `URL.createObjectURL` behavior. |
| `Sidebar` | Render sidebar, pin button, world list, player list. |
| `WorldList` | Render world links, switch worlds. |
| `PlayerList` | Render player links, follow logic, nameplates. |
| `PlayerMarkerLayer` | Render and update rotated player markers on the map. |
| `MarkerLayer` | Render world marker layers (rectangles, polylines, polygons, circles, ellipses, icons). |
| `CoordinatesControl` | Custom Leaflet control showing cursor coordinates. |
| `LinkControl` | Custom Leaflet control copying the shareable URL. |

### Hooks

- `useSettings()` — load and provide `settings.json`.
- `useInterval(callback, delay, enabled)` — polling for players and markers.
- `useMapView()` — derive current world, center, zoom, URL params.
- `usePlayerData(worldName)` — poll `tiles/{world}/players.json`.
- `useMarkerData(worldName)` — poll `tiles/{world}/markers.json`.

### Data flow

1. `App` fetches `tiles/settings.json`.
2. On success, `MapContext` initializes worlds and loads the initial world.
3. `MapView` creates the Leaflet map and mounts the current world’s tile layer.
4. `usePlayerData` / `useMarkerData` start per-world polling.
5. Player list updates the sidebar and map marker layer.
6. Marker JSON updates `MarkerLayer` overlays.
7. URL params are read on init and updated on map move/world change.

### Preserved behavior

- Same coordinate projection (`L.CRS.Simple`, scale based on `zoom.max`).
- Same tile URL pattern: `tiles/{world}/{z}/{x}_{y}.png`, tile size 512.
- Same tile refresh logic: two tile layers swapped after `load` events.
- Same player tracker: head URLs, nameplates, health/armor bars, yaw rotation.
- Same marker types: rectangle, polyline, polygon, circle, ellipse, icon.
- Same sidebar interaction: hover, pin, world/player lists.
- Same URL param behavior: `?world=...&zoom=...&x=...&z=...`, `uuid`, `show_*` toggles.
- Same HMR cleanup: on hot dispose, remove sidebar and map.

## Build & Tooling Changes

### Dependencies to add

- `react` `^19.0.0`
- `react-dom` `^19.0.0`
- `react-leaflet` `^5.0.0` (or latest compatible with React 19)
- `@types/react` `^19.0.0`
- `@types/react-dom` `^19.0.0`
- `typescript` `^7.0.2`
- `@vitejs/plugin-react` `^6.0.5` (latest compatible)

### Existing dependencies to keep

- `leaflet` `^1.9.4`
- `@types/leaflet` `^1.9.21`
- `vite` `^8.1.5`

### Configuration

- `vite.config.ts`: import `@vitejs/plugin-react`, add to plugins.
- `tsconfig.json`: set `jsx` to `react-jsx` and add `react`/`react-dom` types.
- `package.json` scripts remain (`dev`, `build`, `preview`, `format`, `lint`).
- Update `oxlint`/`oxfmt` ignore patterns as needed for new JSX/TSX files.

## Directory Layout

```
web/src/
  main.tsx
  App.tsx
  index.css (renamed from css/styles.css)
  components/
    MapView.tsx
    Sidebar.tsx
    WorldList.tsx
    PlayerList.tsx
    PlayerMarkerLayer.tsx
    MarkerLayer.tsx
    CoordinatesControl.tsx
    LinkControl.tsx
  context/
    MapContext.tsx
  hooks/
    useSettings.ts
    useInterval.ts
    usePlayerData.ts
    useMarkerData.ts
  layers/
    SquaremapTileLayer.tsx
  util/
    Markers.ts
    converters.ts (coordinate / projection helpers)
  addons/
    Ellipse.ts
    RotateMarker.ts
  types.ts (expand existing)
```

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| `react-leaflet` v5 API differences from current imperative Leaflet code | Build small wrapper components and keep direct Leaflet access in `useMap` where necessary. |
| Custom `SquaremapTileLayer` `fetch`/object-URL behavior may not map 1:1 to `react-leaflet` | Implement as a custom `TileLayer` component using `L.TileLayer.extend` and pass it to `react-leaflet` via `createElement`. |
| HMR with React + Leaflet | Replicate current `import.meta.hot.dispose` cleanup in `App` or `MapView`. |
| TypeScript 7 strictness | Update `types.ts` and component prop types; use `satisfies`, `noUncheckedIndexedAccess`. |

## Acceptance Criteria

- `bun run build` succeeds with no type errors.
- `bun run lint` passes (or only warns on acceptable existing issues).
- `bun run dev` serves the React app at `http://localhost:5173/`.
- With a backend present, the map renders tiles, world list, player list, and markers as before.
- Without a backend, the app loads to the same empty-map state as the current JS version.
- All original JS files are removed or renamed to TS/TSX.
