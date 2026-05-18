# stackchan-web

Operator dashboard for the Stack-chan firmware HTTP control plane. TypeScript
+ [Solid](https://www.solidjs.com/) + [Vite](https://vite.dev/), bundled into
a single self-contained `index.html` that the firmware embeds at compile time
under `Content-Encoding: gzip`.

## Build

```bash
just web-build         # install + typecheck + bundle + gzip
just web-typecheck     # tsc --noEmit, no bundle
```

The firmware recipes (`just check-firmware`, `just build-firmware`,
`just clippy-firmware`) chain through `just web-build` because
`crates/stackchan-firmware/src/net/http.rs` `include_bytes!`s
`web/dist/index.html.gz` — the file must exist before `cargo` parses
the macro.

## Develop

```bash
cd web && npm run dev
```

Vite dev server proxies `/state`, `/health`, `/settings`, `/emotion`,
`/look-at`, `/reset`, `/speak`, `/volume`, `/mute`, `/camera*` to
`http://stackchan.local`. With the device on the LAN, the dashboard runs
locally at `http://localhost:5173/` against live firmware.

## Layout

```
src/
  main.tsx                  - entry, mounts <App />
  App.tsx                   - shell: Sidebar + StatusBar + section pages + hotkeys
  nav.ts                    - section ids, hash-router signal, nav metadata
  styles.css                - tactical theme (Stack-chan physical DNA), sidebar + status bar
  types.ts                  - AvatarSnapshot / Settings shapes
  auth.ts                   - localStorage Bearer token + authedFetch wrapper
  store.ts                  - Solid signals: snapshot, conn, toast
  components/
    Sidebar.tsx             - section nav, brand, link status
    StatusBar.tsx           - sticky telemetry strip (face + emotion + pose + battery + wifi + audio)
    FaceGlyph.tsx           - schematic SVG mirroring the LCD face from /state
    ConnStatus.tsx          - SSE connection dot (legacy, now also in Sidebar foot)
    State.tsx               - Status page detail rows
    Emotion.tsx             - emotion presets + hold + reset
    LookAt.tsx              - pan/tilt sliders
    Audio.tsx               - volume + mute, debounced
    Settings.tsx            - GET/PUT /settings form
    Toast.tsx               - transient feedback strip
```

## Navigation

Sections are grouped by subsystem and routed via `location.hash` (e.g.
`#motion`). The sidebar reflects + drives section state; on mobile the rail
collapses to a horizontal scrolling strip.

Hotkeys: `1-9 0 q w e` set emotion, `r` resets, `m` toggles mute, `g <s/b/m/v/y/d/c>`
jumps between sections, `?` opens the shortcut overlay, `Esc` closes.

## Output

`vite build` (via `vite-plugin-singlefile`) inlines JS + CSS into
`dist/index.html`; the build script then runs `gzip -9 -k` to produce
`dist/index.html.gz` alongside it. Both files are gitignored — CI rebuilds
the bundle on every job.
