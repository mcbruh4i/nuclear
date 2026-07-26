# Android Toolchain Setup (Windows) — reproducible log

Every step actually executed on this machine to get from a bare Windows 11 box to an Android-capable Tauri build environment. Steps marked ⛔ were blocked; the workaround used instead is documented.

## Already present (verified, not installed by this run)

- Node.js v25.5.0 (`C:\Program Files\nodejs`)
- JDK 21 — Eclipse Temurin 21.0.7 (`C:\Program Files\Eclipse Adoptium\jdk-21.0.7.6-hotspot`). Tauri Android requires JDK 17+; 21 works.
- git, gh CLI (authenticated as `mcbruh4i`)

## 1. pnpm

Node 25 no longer bundles corepack, so:

```powershell
npm install -g pnpm@10.33.4   # version matches "packageManager" in package.json
```

## 2. Rust — via rustup, GNU host toolchain

⛔ **MSVC path blocked:** `winget install Microsoft.VisualStudio.2022.BuildTools` (C++ workload) failed with installer exit 1602 — the shell is not elevated and no one was present to approve UAC. The default `x86_64-pc-windows-msvc` Rust host toolchain needs those build tools.

**Workaround used:** the self-contained GNU host toolchain. rustup's `rust-mingw` component ships the mingw linker/CRT objects, so host artifacts (build scripts, proc-macros) link without any system compiler. Android targets never touch MSVC — they link with NDK clang.

```powershell
# download https://win.rustup.rs/x86_64 → rustup-init.exe, then:
.\rustup-init.exe -y --default-host x86_64-pc-windows-gnu --default-toolchain stable --profile minimal
# new shells get %USERPROFILE%\.cargo\bin on PATH automatically; current shell needs it added manually

rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
```

> **Follow-up for desktop builds:** install VS 2022 Build Tools (C++ workload) from an elevated shell and `rustup set default-host x86_64-pc-windows-msvc` if you ever build the *Windows* desktop app on this machine. Not needed for Android.

## 3. Android SDK + NDK — user-local, no elevation needed

Installed under `%LOCALAPPDATA%\Android\Sdk` (per-user; `sdkmanager` needs no admin rights).

```powershell
# 1) command-line tools zip (Google's pinned URL, see below for the exact one used)
#    unzip so the tools land at: %LOCALAPPDATA%\Android\Sdk\cmdline-tools\latest\
# 2) accept licenses and install components:
$env:ANDROID_HOME = "$env:LOCALAPPDATA\Android\Sdk"
& "$env:ANDROID_HOME\cmdline-tools\latest\bin\sdkmanager.bat" --licenses   # piped 'y' to all
& "$env:ANDROID_HOME\cmdline-tools\latest\bin\sdkmanager.bat" `
    "platform-tools" "platforms;android-34" "build-tools;34.0.0" "ndk;27.1.12297006"
```

License acceptance: `sdkmanager --licenses` was answered `y` programmatically — installing via sdkmanager is impossible without it and the unattended-run instructions explicitly requested sdkmanager-based installation.

## 4. Environment variables

Set at **user** scope (survives reboots, no elevation needed) and in the current session:

```powershell
[Environment]::SetEnvironmentVariable('ANDROID_HOME', "$env:LOCALAPPDATA\Android\Sdk", 'User')
[Environment]::SetEnvironmentVariable('NDK_HOME', "$env:LOCALAPPDATA\Android\Sdk\ndk\27.1.12297006", 'User')
[Environment]::SetEnvironmentVariable('JAVA_HOME', 'C:\Program Files\Eclipse Adoptium\jdk-21.0.7.6-hotspot', 'User')
# PATH additions (user scope): %USERPROFILE%\.cargo\bin ; %ANDROID_HOME%\platform-tools ; %ANDROID_HOME%\cmdline-tools\latest\bin
```

## 5. Emulator (for on-device DoDs)

```powershell
sdkmanager "emulator" "system-images;android-34;google_apis;x86_64"
avdmanager create avd -n nuclear-test -k "system-images;android-34;google_apis;x86_64" --device pixel_7
# headless boot:
emulator -avd nuclear-test -no-window -no-audio -no-boot-anim -gpu swiftshader_indirect
```

(If hardware acceleration is unavailable — WHPX/Hyper-V off and no admin to enable it — the x86_64 image runs unusably slow or not at all; STATUS.md records what actually happened.)

---

*This file is updated as steps complete; anything not struck through above was executed exactly as written. Exact versions/URLs of what was downloaded are appended below as they are pinned.*

## Appendix: exact artifacts used

| Artifact | Source | Notes |
|---|---|---|
| rustup-init.exe | https://win.rustup.rs/x86_64 | installed rustc/cargo 1.97.1, host `x86_64-pc-windows-gnu`, profile minimal |
| Android cmdline-tools | https://dl.google.com/android/repository/commandlinetools-win-11076708_latest.zip | 153,583,359 bytes; extracted to `%LOCALAPPDATA%\Android\Sdk\cmdline-tools\latest` |
| pnpm | `npm install -g pnpm@10.33.4` | |
| SDK components | `sdkmanager "platform-tools" "platforms;android-34" "build-tools;34.0.0" "ndk;27.1.12297006"` | licenses pre-accepted via stdin-redirect (`--licenses < yes.txt`; plain PowerShell y-piping does NOT reach the JVM) |

### Gotcha log

- `sdkmanager --licenses` ignores y's piped from PowerShell (`"y"*n | sdkmanager.bat`); the JVM never sees them. Working form: `cmd /c "sdkmanager.bat --licenses < yes.txt"` with a CRLF `y` file.
- Env vars were set at **user** scope: `ANDROID_HOME`, `NDK_HOME`, `JAVA_HOME`, plus PATH additions (`%USERPROFILE%\.cargo\bin`, `platform-tools`, `cmdline-tools\latest\bin`). Open shells need a restart to see them.
