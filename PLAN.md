# Android Port Plan — Nuclear × Tauri 2 Mobile

Analysis of `nukeop/nuclear` at `player@1.43.2` (d030cc1) for porting to Android using Tauri 2's mobile support. All file references are relative to the repo root and were verified against this revision.

A key framing fact up front: **Nuclear v2 is already a Tauri 2.7 app** (`packages/player/src-tauri/Cargo.toml:23`). This is not an Electron migration — it is adding a mobile target to an existing Tauri app. Upstream has even started: `#[cfg_attr(mobile, tauri::mobile_entry_point)]` is already on `run()` (`src-tauri/src/lib.rs:70`) and the desktop-only plugins are already Cargo-gated off mobile (`Cargo.toml:55-60`). The work was left unfinished — see §7 for the verified compile breaks.

---

## 1. Architecture overview

### Monorepo layout

pnpm workspaces (`pnpm-workspace.yaml`: single glob `packages/*`) + Turborepo (`turbo.json`). Thirteen packages:

| Package | Purpose | Runtime |
|---|---|---|
| `player` | The app: React 19 + Vite 7 + TanStack Router + Tailwind v4 + Zustand 5, over a Rust/Tauri backend (`src-tauri/`, 33 files, ~6k lines) | app |
| `hifi` | Audio playback engine: hidden `<audio>` element + hls.js + hand-rolled fMP4/MSE pipeline | browser |
| `model` | Domain types + zod schemas (Track, Album, Playlist, Stream…) | pure TS |
| `plugin-sdk` | Host-pattern facade classes plugins compile against; published to npm | pure TS |
| `i18n` | i18next singleton, statically bundled locale JSON | pure TS |
| `themes` | Theme engine: CSS-variable basic themes + validated JSON→CSS "advanced" themes | DOM |
| `tailwind-config` | CSS-first Tailwind v4 design tokens (`global.css` `@theme` block) | CSS |
| `ui` | 57-component presentational library; **zero `@tauri-apps` imports** | browser |
| `tools` | Release scripts (Node, build-time only) | CI |
| `docs`, `storybook`, `website`, `eslint-config` | Docs/dev tooling, not shipped | — |

### How Rust and React communicate

Three overlapping mechanisms (the codebase is mid-migration):

1. **Generated bindings** — `tauri-specta` exports all 26 commands + types to `packages/player/src/services/tauri/bindings.ts` at debug-build time (`src-tauri/src/lib.rs:76-85`). Only the History feature actually consumes them (`services/history/historyService.ts:10-11`).
2. **Raw `invoke`** — everything else: 21 call sites across `services/tauri/commands.ts`, `httpHost.ts`, `ytdlpHost.ts`, `services/mcp|mpd|httpApi|discordHandler`, `streamResolution/audioSource.ts:38`, `bridge/*`, `hooks/useLogStream.ts:83`.
3. **The Bridge (Rust→JS RPC)** — `src-tauri/src/bridge/bridge.rs` emits a `bridge:request` Tauri event with a UUID and waits on a oneshot (30 s timeout); JS answers via the `bridge_respond` command (`services/bridge/bridgeHandler.ts:55`, `bridgeDispatcher.ts:26-32`). This inversion lets *Rust* call the *plugin API* — it is what powers MCP, MPD, and the LAN remote.

### The four localhost servers

| Server | Bind | Ports | Role |
|---|---|---|---|
| **Stream proxy** | `127.0.0.1` | 9100-9109 | **Load-bearing for playback.** `<audio>` never fetches upstream URLs directly; `audioSource.ts:45-51` base64url-encodes the URL and plays `http://127.0.0.1:{port}/stream/{encoded}`, bypassing CORS (`src-tauri/src/stream_server.rs`). Always started at setup. |
| MCP | `127.0.0.1` | 8800-8809 | `rmcp` streamable-HTTP server (`src-tauri/src/mcp/`), 4 tools routed through the Bridge into the JS plugin API. |
| MPD | `127.0.0.1` | 6600-6609 | MPD 0.25 protocol server (`src-tauri/src/mpd/`), also Bridge-backed. |
| Jam remote | `0.0.0.0` | 4120-4129 | LAN remote control: REST + SSE + a `rust_embed` copy of the whole built frontend (`src-tauri/src/http_api/frontend.rs:9`). |

### Audio: zero in Rust

No rodio/cpal/symphonia anywhere in `src-tauri` (verified by grep). Rust only *byte-forwards* streams (`stream_server.rs:112`). All decoding/playback is WebView-side in `packages/hifi`: `Sound.tsx` renders a hidden `<audio>`, `useHlsSource.ts` (hls.js), `useMseSource.ts` + `src/fmp4/*` (hand-rolled fMP4→MediaSource pipeline). The state machine driving it, `packages/player/src/stores/soundStore.ts`, is 75 lines of pure Zustand with zero platform imports.

---

## 2. Reusability audit

### REUSABLE — platform-agnostic, usable on Android as-is

| Module | Evidence |
|---|---|
| `packages/model` | Pure TS + zod; tests run in `environment: 'node'` (`model/vite.config.ts:27`). |
| `packages/i18n` | Locales statically imported (`i18n/src/i18n.ts:4-20`); no fs, no detection (`lng` hardcoded — see §3). |
| `packages/themes` | DOM-only: sets `data-theme-id` on `<html>`, injects a `<style>` tag (`themes/src/index.ts:85-108`). Themes bundled as CSS; zod-validated JSON→CSS generator is pure. |
| `packages/tailwind-config` | Pure CSS design tokens. (Needs a `@source` line added if a new mobile package appears — `global.css:7-9`.) |
| `packages/plugin-sdk` | Pure host-pattern facades; zero platform code (only hit for node/tauri/electron across `src/` is a comment at `src/types/ytdlp.ts:1`). |
| `stores/soundStore.ts` | The audio state machine: pure, 75 lines, imports only `eventBus`/`Logger`/`secondsToMs`. **Ports for free.** |
| `stores/queueStore.ts` (logic) | All index math is pure: `getDirectionalIndex` (:56-85), `getShuffledIndex` (:87-98), the `produce()` reducers. Persistence is the only Tauri touch (see ADAPTABLE). |
| `stores/settingsModalStore.ts`, `startupStore.ts`, `historyStore.ts` | Pure, verified. |
| `remoteControl/remoteStore.ts` | 100 % pure setters — designed to be fed from any source (today: SSE). |
| `services/playbackEventBridge.ts` | Pure store subscription. |
| `services/coreSettings.ts` | Declarative settings schema (30 `SettingDefinition`s) rendered generically — a mobile settings screen reuses it wholesale. |
| The `Connected*` pattern | `packages/ui` receives state as props and actions as callbacks; all binding lives in `packages/player/src/components/Connected*`. The pattern itself is the portability seam. |
| `packages/ui` leaf components | `Button`, `Input`, `Textarea`, `Slider` (real `<input type="range">` — touch-OK), `Card`, `CardGrid` (already `auto-fit` responsive), `Select`, `Tabs`, `Dialog`, `Toggle`, `Badge`, `Loader`, `EmptyState`, `Toaster`, `FavoriteButton`, `FilterChips`, `Mosaic`, `ImageReveal`, `StatChip`, `SectionShell`, `ViewShell`, `RouteTransition`. |
| `packages/ui/src/components/NuclearJam/` | 14 phone-shaped components (`h-dvh`, single column, large controls) — see §4. |

### ADAPTABLE — core logic survives, needs changes

| Module | What changes |
|---|---|
| `packages/hifi` | Works in Android WebView as-is (Chromium: `<audio>`, MSE, hls.js, Range-fetch all fine). But it becomes **desktop-only on the mobile path** once a native playback backend exists (§4) — WebView audio cannot survive backgrounding. One latent bug either way: `LoggerProvider.get()` returns `undefined` until the host calls `init()`, and `MseController.handleSetupFailure` (`hifi/src/fmp4/MseController.ts:74`) calls it unguarded — the mobile entrypoint must call `init()` like the desktop one does. |
| The `LazyStore` persistence pattern | Identical hand-rolled `saveToDisk()`/`withPersistence()` in five stores (`queueStore.ts:106-124`, `favoritesStore.ts:40-46`, `shortcutsStore.ts:21-38`, `providersStore.ts:19-23`, `settingsStore.ts:33-69`). `@tauri-apps/plugin-store` does support Android, so this may work unchanged — but extracting a single persistence port is cheap and makes the seam explicit. |
| `stores/playlistStore.ts` | Already a clean facade over `services/playlistFileService` (the only place touching `plugin-fs`); inject a mobile file service. |
| `stores/themeStore.ts` | Logic fine; the disk-watcher it relies on (`advancedThemeWatcher.ts`, `watchImmediate`) is unreliable on Android — disable hot-reload, keep load-on-demand. |
| Plugin install/load (`services/plugins/*`) | `$APPDATA`-rooted paths (`pluginDir.ts:10-30`, `pluginDownloader.ts:7-33`) map to Android app-private storage; the Rust primitives (`download_file`, `extract_zip`, `copy_dir_recursive` in `commands.rs`) cross-compile. Three real issues: (a) `esbuild-wasm` (~10 MB) initialized on the main thread with `worker: false` (`pluginCompiler.ts:118`) — heavy on mobile; prefer pre-compiled JS plugins (loader already supports that path, `PluginLoader.ts:111-113`); (b) plugin execution is `new Function(...)` (`PluginLoader.ts:134`) requiring `unsafe-eval` — fine today because CSP is `null` (`tauri.conf.json:25`), constrains any future CSP tightening; (c) dev-plugin folder-picker has no Android equivalent — drop on mobile. |
| `src-tauri`: `http.rs`, `stream_server.rs`, `history/*` + sqlx, `bridge/*`, `commands.rs`, `db.rs`, `logging.rs`, `net.rs` | All portable. sqlite needs NDK cross-compile config; reqwest should pin `rustls-tls` for Android. |
| `capabilities/default.json` | `$APPDATA`-scoped fs permissions nominally resolve on Android; `fs:allow-watch` entries (:94-106) and `opener:allow-reveal-item-in-dir` (:155-165, no Android impl) need a mobile-specific capability file. |
| Frontend boot | `main.tsx:8-18` already forks on `__TAURI_INTERNALS__`; `initPlayerApp.tsx` is a linear list of ~20 initializers — an `initMobileApp` sibling skips MCP/MPD/HTTP-API/Discord/updater/yt-dlp cleanly. |

### DESKTOP-ONLY — replace or drop

| Module | Why |
|---|---|
| `src-tauri/src/ytdlp.rs` + `ytdlp_setup.rs` | Downloads a yt-dlp binary into app-data and `exec`s it (`ytdlp.rs:86-101`). **Blocked on Android by W^X since API 29** — you cannot execute downloaded binaries. See §3: this is the port's central blocker, not a peripheral feature. |
| `src-tauri/src/discord.rs` | Discord IPC socket/named pipe — cannot exist on Android. |
| `src-tauri/src/mpd/*` | MPD clients connect over LAN to a desktop; pointless on a phone. |
| `src-tauri/src/http_api/*` | The Jam *server* (phone as remote-control target) is out of scope; also `rust_embed` of `../dist/` (`frontend.rs:9`) would double APK size. The phone is the Jam *client* — that's the existing `remoteControl/` app. |
| `src-tauri/src/mcp/*` | Localhost MCP server has no client on a phone. |
| `fix_path_env` (`main.rs:5`) | Spawns the user's login shell to harvest PATH — desktop-only concept; must be `#[cfg(desktop)]`. |
| `stores/updaterStore.ts` | `plugin-updater`/`plugin-process`, already Cargo-gated off mobile; Android updates come from the store/APK channel. |
| `shortcuts/*`, `views/KeyboardShortcuts`, `ui/KeyCombo` | Keyboard-only. Cleanly isolated; excluded on mobile. |
| `ConnectedTitleBar.tsx`, `hooks/useFramelessWindow.ts`, `ui/TitleBar/*` | Window decorations; the window-control invokes are already permission-scoped to desktop platforms (`capabilities/desktop.json`) so they'd throw at runtime on Android. |
| `ui/PlayerWorkspace/*` | Fixed 3-column grid, mouse-only resize (see §4). |
| `ui/TrackTable/*` (24 files), `TrackContextMenu/*`, `QueueItemPopover/*`, `Popover/ContextMenuTrigger.tsx`, `Tooltip/*`, `SettingsPanel/*`, `LogViewer/*`, `Slider/useSliderWheel.ts`, `ScrollableArea` | 8-column fixed tables, right-click-only triggers, hover-only affordances, mouse-enter tooltips, wheel handlers — all desktop input assumptions. |

### Verified compile breaks (Android target)

Confirmed by direct read of `src-tauri/src/lib.rs`:
1. `lib.rs:94` — `.plugin(tauri_plugin_window_state::…)` unconditional, but the crate exists only for desktop targets (`Cargo.toml:59-60`). Unresolved crate on Android.
2. `lib.rs:97-101` — `tauri_plugin_updater` + `tauri_plugin_process` behind a *runtime* `if !is_flatpak`, not a `#[cfg]`; crates gated at `Cargo.toml:55-57`. Same failure.
3. `ytdlp_setup.rs:32-76` — `release_filename()` / `binary_name()` have per-OS `#[cfg]` arms with no Android arm → no return value on that target.

(Compile-probe result recorded in §7.)

---

## 3. Android-specific gaps

Everything an Android music player needs that this codebase does not have:

| Gap | Current state | Tauri story |
|---|---|---|
| **Background audio playback** | None. Audio lives in the WebView; Android pauses WebView audio on backgrounding. | No official plugin. Community: [`tauri-plugin-native-audio`](https://github.com/uvarov-frontend/tauri-plugin-native-audio) (Media3 ExoPlayer + `MediaSessionService` + foreground service, `FOREGROUND_SERVICE_MEDIA_PLAYBACK`). Third-party and unvalidated — see §7. Fallback: custom Kotlin `MediaSessionService`. |
| **Media session / notification / lockscreen controls** | Zero `navigator.mediaSession` usage anywhere (verified by grep). | Same plugin, or custom Kotlin MediaStyle notification. Even a WebView-audio interim could add `navigator.mediaSession` metadata cheaply, but real controls need the native service. |
| **Audio focus** | None. | Free with Media3 (`setAudioAttributes(…, handleAudioFocus = true)`); manual `AudioFocusRequest` Kotlin otherwise. |
| **Battery-optimization exemption** | None. | No plugin exists; custom Kotlin (`ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS` intent) if Doze kills the service. |
| **Storage access (shared/local media)** | All fs is `$APPDATA`-scoped, which maps to app-private storage and works. Local-file *library* features would need shared storage. | `tauri-plugin-fs` covers app-private only. SAF/MediaStore → community [`tauri-plugin-android-fs`](https://github.com/aiueo13/tauri-plugin-android-fs); note upstream `FilePath` can't enumerate a directory from a `content://` URI ([tauri#14587](https://github.com/tauri-apps/tauri/issues/14587)). Not needed for streaming-only M0-M2. |
| **Notification permission (API 33+)** | None. | Official `tauri-plugin-notification` handles the runtime permission. |
| **Cleartext localhost** | Playback depends on `http://127.0.0.1:{port}/stream/…` (`audioSource.ts:50`). Android blocks cleartext by default. | Config, not code: `network-security-config` exempting `127.0.0.1` (preferred over blanket `usesCleartextTraffic`). |
| **Stream resolution — THE CENTRAL BLOCKER** | `packages/player/src/services` ships **no built-in providers**; every provider is a marketplace plugin, and stream resolution reaches plugins as a *host capability* (`plugin-sdk/src/api/ytdlp.ts`) backed by spawning the yt-dlp binary — impossible on Android (W^X, API 29+). Without a replacement, an Android build plays nothing. | Three options behind the **unchanged** `Ytdlp` command surface (`ytdlp_search` / `ytdlp_get_stream` / `ytdlp_get_playlist`): **(a) pure-Rust InnerTube extractor (e.g. `rustypipe`) — recommended**: no process spawn, works offline-first, keeps the plugin contract intact; (b) Invidious/Piped public APIs via the existing `http_fetch` — zero Rust work but depends on third-party instance uptime; (c) ship `Ytdlp.available = false` (the API already models absence, `api/ytdlp.ts:15-17`) — honest but yields an app with no content. De-risked as milestone M2 (§6) before any UI investment. |
| **Mobile capability file** | `os:default` lives only in `capabilities/desktop.json` while `App.tsx:28` calls `platform()` unconditionally → denied on Android. | Add `capabilities/mobile.json` (platforms `["android","iOS"]`) with `os:default` + the `$APPDATA` fs scopes minus watch/reveal. |
| **Config/bundle gaps** | No `bundle.android` block, no Android icons, `bundle.targets: "all"` + `createUpdaterArtifacts: true`, `devUrl: localhost:5173` (device needs `VITE_HOST` — the existing `dev:remote` script already sets `VITE_HOST=0.0.0.0`), `Platform` union lacks `'android'` (`ui/src/providers/PlatformProvider.tsx:3`). | All straightforward config/type additions. `tauri android init` generates `gen/android/`. |
| **Locale detection** | `lng` hardcoded `'en_US'` (`i18n/src/i18n.ts:25`). | Add `navigator.language` detection on mobile. |

---

## 4. UI strategy

### How coupled is the current UI to desktop?

Completely — by the numbers (verified by grep across the monorepo):

- **0** `@media` queries. **0** `matchMedia` calls. **4** real responsive Tailwind utilities in total (three `md:flex-row` headers + one `sm:` in `PluginItem`).
- **1** line of touch handling in the entire UI (`QueueItemExpanded.tsx:119`, a `stopPropagation`).
- `PlayerWorkspace` is a fixed 3-column grid (`grid-cols-[auto_1fr_auto]`); its two sidebars consume 108 px *fully collapsed* (`constants.ts: COLLAPSED_WIDTH: 54`) — 27 % of a 400 px screen before content starts.
- `TrackTable`: 8 fixed columns, `table-fixed`. Settings modal: `w-[80vw]` with a `w-56` fixed nav. `PlayerShell`: `h-screen` (not `h-dvh` — fights Android browser chrome).
- Touch-unreachable functionality: queue-remove is `opacity-0 group-hover:opacity-100`; the stream-candidate picker opens only via right-click (`ContextMenuTrigger.tsx:9-13`); tooltips are `onMouseEnter`-only; sidebar resize is `document.addEventListener('mousemove')`.

Roughly **25-30 % of the UI survives** — and it is precisely the leaf components (§2 REUSABLE list). Every composite layout is desktop-shaped.

### Recommendation: build a new mobile shell that imports the shared logic/state layer

Do **not** retrofit responsiveness into `PlayerWorkspace`/`TrackTable`/`SettingsPanel`. The three reasons:

1. **The state layer is the asset, and it's clean.** `soundStore` is pure; `queueStore`'s logic is pure; `packages/ui` has zero Tauri imports; all platform binding already lives in swappable `Connected*` wrappers. The architecture was accidentally built for this.
2. **A mobile UI already exists in-tree.** `packages/ui/src/components/NuclearJam/` (14 components: `NuclearJamRoot` with `h-dvh` single-column, `NowPlaying`, `Controls`, `Queue`, …) is the phone-shaped remote-control UI, driven by `remoteControl/` + the pure `remoteStore`. The mobile app is architecturally "NuclearJam pointed at local stores instead of SSE."
3. **The boot fork already exists.** `main.tsx:8-18` picks an entrypoint at runtime; `initMobileApp` becomes a third sibling that initializes only mobile-relevant services.

Concretely: new mobile views composed from existing leaf components + NuclearJam patterns; `TrackTable` → single-column track list (virtualized with the already-present `@tanstack/react-virtual`); context menus → long-press bottom sheets; settings modal → full-screen page reusing the `coreSettings.ts` schema; `SidebarNavigation` → bottom tab bar.

**Playback backend seam:** keep `soundStore` as the platform-agnostic state machine and introduce a `PlaybackBackend` port with two implementations — `HifiBackend` (desktop, the current `<Sound>`/`SoundProvider` path) and `NativeBackend` (Android Media3 via plugin or custom Kotlin). This is the only route to background playback, lockscreen controls, and audio focus; it demotes `packages/hifi` (including its fMP4/MSE stack, which ExoPlayer supersedes) to desktop-only on the mobile path.

### Design-tokens rule (hard requirement, every mobile component, forever)

The mobile shell consumes design tokens **exclusively**:

- All colors via the CSS variables defined in `packages/themes` (mapped through the `@theme` block in `packages/tailwind-config/global.css:11-56`) — i.e. Tailwind utilities like `bg-background`, `text-primary`, never `bg-[#1a1a2e]`.
- Spacing and typography via `packages/tailwind-config` tokens.
- **No raw hex values. No hardcoded `font-family` declarations. No inline px font sizes.**

This is a rule, not a suggestion. It is what makes the entire existing theme system — bundled basic themes, advanced JSON themes, marketplace themes, custom user themes — work on Android for free: themes only swap CSS variables (`themes/src/index.ts:85-108`), so any component written against tokens is automatically themeable.

---

## 5. Recommended repo strategy: fork (not a fresh repo importing core packages)

The fresh-repo-with-dependencies model fails on the facts of the audit:

1. **The reusable logic isn't packaged.** The most valuable portable code — `stores/`, `services/coreSettings.ts`, `playbackEventBridge`, the stream-resolution services — is app code inside `packages/player`, not an importable package. A fresh repo would have to vendor it (instant permanent divergence) or first drive a large extraction refactor upstream (slow, and hostile to a project you don't maintain).
2. **You can't npm-install what you need.** Only `@nuclearplayer/plugin-sdk` has a publish workflow, and it *strips* `@nuclearplayer/ui` and `model` from its dependencies on publish (`.github/workflows/release-plugin-sdk.yml:72-73`). `ui`, `hifi`, `themes`, `i18n` are unpublished workspace packages.
3. **The mobile app needs most of the existing Rust.** Bridge, `http_fetch`, `stream_server`, history/sqlx, the fs/store plumbing — all reused as-is (§2). A fresh repo duplicates ~6k lines of Rust that upstream will keep evolving.
4. **Upstream is active and mobile-sympathetic.** 1.43.2, regular releases, and the mobile gating in `Cargo.toml`/`lib.rs:70` shows intent. Staying rebasable captures that work; a snapshot forfeits it.

**Working model:** fork under your account with `upstream` → `nukeop/nuclear`; branch `android-port`. Keep changes **additive and platform-gated** so rebases stay trivial: new files (`initMobileApp.tsx`, mobile views, `capabilities/mobile.json`, `gen/android/`, `bundle.android` block, `tauri android` scripts) plus narrow `#[cfg(desktop)]` / platform-check edits at the seams. Add `gen/android` to the eslint ignores (`packages/eslint-config/eslint.config.ts:14-25`) and a `@source` line to `tailwind-config/global.css` if a mobile package is added. Then **upstream the seams themselves** — the `PlaybackBackend` port, the persistence port, the `Platform` union gaining `'android'`, the `cfg` fixes — as small PRs; every one accepted shrinks the fork.

**Decisions.** The backend stays Rust/Tauri. A separate server backend (e.g. FastAPI) was considered and rejected for this port: it would change the product from an offline-first app into a client-server service, add hosting costs and legal exposure, and discard the reusable Rust already validated by this audit (bridge, `http_fetch`, `stream_server`, history). A remote companion service may be revisited later for cross-device sync, as a separate project.

---

## 6. Milestone roadmap

Ordered, small, independently testable. **Stream resolution is deliberately M2** — right after "app shows a screen" — because it is the go/no-go gate for the whole project and must be de-risked before any UI investment.

- **M0 — Toolchain + skeleton builds.** Android SDK/NDK/JDK + Rust android targets; `tauri android init` generates `gen/android/`; fix the three verified compile breaks (`#[cfg(desktop)]` around `lib.rs:94` and `lib.rs:97-101`, gate the ytdlp modules + their command registrations, gate `fix_path_env` in `main.rs`); add `capabilities/mobile.json`.
  **DoD:** debug APK installs and renders *any* screen (even the broken desktop UI) on a device/emulator.
- **M1 — Stream proxy alive on device.** `stream_server` boots on Android; cleartext-localhost network-security-config in place.
  **DoD:** a hardcoded direct audio URL plays through `http://127.0.0.1:{port}/stream/…` in the WebView.
- **M2 — GO/NO-GO: stream resolution without yt-dlp.** Rust InnerTube extractor (rustypipe or equivalent) wired behind the unchanged `Ytdlp` command surface.
  **DoD:** on-device search returns results and a resolved stream plays end-to-end for a handful of tracks, re-verified across several days (extractor breakage is exactly the risk being probed). If this fails: fall back to Invidious/Piped via `http_fetch` **before** touching any UI work.
- **M3 — Mobile shell boots.** `initMobileApp` entrypoint; NuclearJam-derived single-column shell over local `soundStore`/`queueStore`; design-tokens rule enforced from the first component.
  **DoD:** search → tap → play → queue-advance works on-device, foreground.
- **M4 — Native playback backend.** `PlaybackBackend` port; `NativeBackend` via Media3 (community plugin if it passes validation in §7, else custom Kotlin).
  **DoD:** playback survives screen-off and backgrounding; audio focus honored (pauses on interruption, resumes appropriately).
- **M5 — Media session + notification.** MediaStyle notification (play/pause/next/prev), lockscreen controls, headset/Bluetooth keys; API 33+ notification-permission flow.
  **DoD:** transport controls work from lockscreen and shade.
- **M6 — Persistence + settings parity.** Queue/favorites/settings/playlists persisted via the store files on Android; desktop-only settings hidden; history recording (sqlx/SQLite).
  **DoD:** kill + relaunch restores queue position, favorites, settings.
- **M7 — Plugins on mobile (scoped).** Marketplace install of pre-compiled JS plugins; esbuild-wasm deferred/off-main-thread or skipped on mobile; dev-plugin folder-picker dropped.
  **DoD:** at least one provider plugin installs from the registry and resolves content on-device.
- **M8 — Core-playback parity + release plumbing.** Themes (bundled + marketplace) applying; i18n with `navigator.language`; CI matrix entry producing a signed APK.
  **DoD:** tagged build produces an installable signed APK with the M0-M7 feature set.

---

## 7. Risks and unknowns

Things not verifiable by reading code, each with the experiment that resolves it.

| Risk | Why it's open | Resolving experiment |
|---|---|---|
| **`tauri-plugin-native-audio` maturity** | Third-party, unvalidated; Media3/MediaSessionService claims are README-level. | Throwaway Tauri app: integrate, play a URL, 30-min screen-off soak, interruption (call/alarm) + Bluetooth tests. Decide plugin-vs-custom-Kotlin **before** M4. |
| **`tauri-plugin-android-fs` maturity** | Third-party; SAF `content://` handling has known upstream friction (tauri#14587). | Only matters for local-file features; spike a pick→read→play round-trip when that's scheduled. Not on the M0-M2 path. |
| **rustypipe (InnerTube) yields playable streams on-device** | Cipher/ABR churn; desktop success ≠ Android success (TLS stack, IP reputation, player-JS differences). | The M2 spike itself: resolve + play one track on emulator *and* physical device; re-run across several days. **This is the go/no-go gate.** |
| **ToS / Play Store policy** | YouTube stream extraction violates YouTube ToS. Acceptable exposure for a personal open-source app distributed as APK/F-Droid-style — but **very likely a rejection/removal reason if ever submitted to Google Play**. | No experiment resolves a policy; it's a distribution decision. Default assumption: sideload / GitHub releases, not Play Store. |
| **WebView `http://127.0.0.1` from the app origin** | Mixed-content/cleartext behavior on Android WebView verified only from docs. | Part of M1's DoD. |
| **esbuild-wasm on mid-range Android** | 10 MB wasm, main-thread init (`pluginCompiler.ts:118`, `worker: false`). | Measure init time on a low-end device during M7; if > ~2 s, require pre-compiled plugins on mobile. |
| **Store-file locking / `LazyStore` semantics under Android lifecycle** | Process death mid-write differs from desktop quit. | Covered by M6's kill-and-relaunch DoD. |
| **Compile-probe result** (`cargo check --target aarch64-linux-android`) | Predicted first errors: `lib.rs:94`, `lib.rs:97-101`, `ytdlp_setup.rs:32-76`. | **Recorded here after the mandatory verification run:** _pending — this line is updated by the verification step in this session (see STATUS.md)._ |
