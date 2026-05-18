# stackchan-companion

A live 3D mirror of the Stack-chan running locally on the sidecar host —
TypeScript + [Vite](https://vite.dev/) + [three.js](https://threejs.org/).
The sidecar's FastAPI process mounts the built bundle under `/companion/`
and relays the firmware's SSE state stream via `/v1/state-proxy`, so the
browser only ever talks to localhost.

This is intentionally separate from the on-device operator dashboard
(under `web/` at the repo root): that one is served from CoreS3 flash and
must stay small; this one runs on the sidecar host, has no flash budget,
and can afford the ~150 KB of three.js.

## Build

```bash
just sidecar-companion-build   # npm install + typecheck + vite build → dist/
```

The sidecar's `register_companion` mount looks for `sidecar/web/dist/index.html`
on startup. If it's missing the static mount is skipped but the SSE relay
endpoint still registers, so `just sidecar-companion-build` is not a hard
prerequisite for running the voice-agent path.

## Develop

```bash
cd sidecar/web && npm run dev
```

Vite serves the page at `http://localhost:5174/` and proxies `/v1/*` to
the sidecar at `http://localhost:8080`. The sidecar in turn proxies
`/v1/state-proxy` from the firmware's `/state/stream`. Configure the
firmware target with `STACKCHAN_FIRMWARE_URL` (default
`http://stackchan.local`).

## Layout

```
src/
  main.ts            - entry: scene + state driver + RAF loop
  scene.ts           - three.js scene (body, head, face decal, status LED)
  face-texture.ts    - 2D-canvas port of the dashboard's schematic face glyph
  state.ts           - SSE client + smoothed pose / idle anim drivers
  styles.css         - HUD + canvas layout
```
