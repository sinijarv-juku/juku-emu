# Juku E5104 emulator — implementation spec

Status: the macOS app, the Windows app (§11) and the guest debugger, served from both apps to VS Code (§12), are built; §8 says what is emulated and what is not. Each part gives the design, and "as built" notes record where the implementation adds to it or differs.

Hardware reference: [Juku_machine_description.md](Juku_machine_description.md), cited below as "MD §n". This spec covers how the emulator is built, not how the machine works.

*Paths to source material (`Schematics/`, `Roms/`, `Software/`) refer to a private archive of Juku documents, ROMs and disks, outside this repository; much of it is public through the juku3000 project (https://github.com/infoaed/juku3000). The ROM images are also in `resources/roms/`. The disk images are not part of the repository.*

---

## 1. Goals and scope

**In scope (v1)**
- A native macOS app with a single window showing the Juku screen through a Metal CRT shader.
- An accurate enough emulation of the production E5104 to boot the supplied RomBios 3.43m ROM to the `*` prompt and boot EKDOS from MAME-format floppy images.
- Two virtual 5.25" drives (A: and B:). Images can be inserted, ejected and saved.
- The keyboard (positional mapping), the speaker beeper (CoreAudio) and the E4701 mouse (captured host mouse).
- Rust for all emulation, with **no macOS dependencies in the core**; Objective-C only for the OS layer.

**Out of scope (v1), with hooks left for later**
- Cassette tape and the early tape-USART board.
- The local network (8251 networking and the F0–F3 coprocessor).
- Printer output.
- The external 32 KB memory pages (MD §3.2) and the RAM disk (MD §3.5).
- The 8" drive mode.
- Front-ends for other platforms. (Since v1: a Windows front-end, §11, and a guest debugger, §12.)

**Accuracy target**
- Exact cycle counts for every 8080 instruction.
- All devices are clocked from the CPU cycle counter, so the timer-driven video timing, the 50 Hz frame interrupt and the beeper pitch come out right without special cases.
- Floppy emulation is at sector level with realistic rotation and byte timing. There is no flux-level emulation.

---

## 2. Project layout

The repository holds the core, which every front-end shares, and one directory per front-end under `frontends/`. Shared data (the ROMs and the UI strings) stays in `resources/`. The build scripts are at the root.

The macOS front-end has no Xcode project: a zsh `build.sh` assembles the `.app` bundle by hand, and Objective-C is compiled as **one translation unit**. `Main.m` `#import`s the other `.m` files, and every `.m` starts with `#pragma once`. There are **no `.h` files** for the Objective-C code itself. The one header, `ShaderTypes.h`, shares types with the separately compiled Metal shaders (below). The one exception is the Quick Look thumbnail extension in `frontends/macos/src/thumbnail/`, a binary of its own (§6.8).

```
juku-emu/
├── VERSION                # the version, semantic: MAJOR.MINOR.PATCH, -dev between releases (§3.3)
├── Cargo.toml, Cargo.lock # the Rust workspace: its crates and profiles (§3.1)
├── .cargo/config.toml     # cargo's output in target/cargo (§3.1)
├── .github/workflows/     # GitHub Actions: ci.yml tests and builds both apps, release.yml drafts a release (docs/Development.md)
├── build.sh               # macOS: builds core and debug server (cargo) → ObjC/Metal → .app bundle
├── run.sh                 # macOS: launches the app with its os_log stream attached
├── package.sh             # macOS: signed release build → target/Juku-<version>.dmg (§3.4)
├── build.cmd              # Windows: cargo with build-std → target\windows\Juku.exe (§11.1)
├── README.md              # for users: features, installation, use, screenshots (docs/images/)
├── docs/                  # this spec, the machine description, Development.md (building, tests, debugging), T3.2_and_TIMCNT.md, images/
├── tools/                 # scpdisk.py: flux dumps (.scp) to sector images (the assembler, disassembler and disk tool are juku-sdk's, ../juku-sdk, §12.6)
├── LICENSE                # MIT
├── .gitignore             # target, .* but .gitignore, .gitattributes, .github and .cargo, floppies, demos/*.dbg
├── core/                  # Rust crate "juku" (OS-agnostic)
│   ├── Cargo.toml
│   ├── src/
│   │   ├── lib.rs         # crate root, re-exports, #![deny(unsafe_op_in_unsafe_fn)]
│   │   ├── ffi.rs         # the C ABI, used by the macOS front-end (§5)
│   │   ├── machine.rs     # Juku: bus, memory modes, scheduler, wiring
│   │   ├── expansion.rs   # expansion ROM check (plug-in memory expander)
│   │   ├── cpu/i8080.rs   # 8080A core
│   │   ├── cpu/disasm.rs  # 8080 disassembler, Intel mnemonics (§12.2)
│   │   ├── debug.rs       # guest debugger: breakpoints, watchpoints, steps, shadow stack (§12.4)
│   │   ├── dev/pic8259.rs
│   │   ├── dev/pit8253.rs
│   │   ├── dev/ppi8255.rs
│   │   ├── dev/usart8251.rs   # minimal stub (status only)
│   │   ├── dev/fdc1793.rs     # КР1818ВГ93
│   │   ├── floppy.rs      # drive mechanics + disk image (.juk)
│   │   ├── keyboard.rs    # 16×6 matrix + 74148 + CONTRDAT DIP banks
│   │   ├── mouse.rs       # E4701 card at port 80h
│   │   ├── snapshot.rs    # snapshot files: state in PNG chunks (§4.6)
│   │   ├── lz.rs          # LZ77 for the disks in snapshots (§4.6)
│   │   ├── video.rs       # the raster the beam draws; geometry from PIT counts
│   │   ├── audio.rs       # speaker → PCM, lock-free SPSC ring
│   │   ├── drive_sound.rs # the floppy drives' sounds, made from noise
│   │   ├── drive_sound/fitted.rs  # their numbers, fitted to recordings
│   │                      # unit tests in each module (cfg(test))
│   └── tests/             # integration tests (boot ROM headless, CPU exercisers, disks, debugging)
├── debugger/              # the guest debugger (§12)
│   └── dap/               # crate juku-dap: the DAP server, with the core the Mac app's Rust library and a crate the Windows app uses (§12.2); the VS Code extension is juku-sdk's (§12.8)
├── frontends/
│   ├── macos/
│   │   ├── compile_flags.txt  # for clangd: -fobjc-arc -fmodules -mmacosx-version-min=14.0
│   │   ├── dmg/DiskImageArt.m # package.sh's disk image artwork: the window's background, the volume icon (§3.4)
│   │   ├── src/               # Objective-C (single TU) + Metal
│   │   │   ├── Main.m         # NSApplication, menus, AppDelegate; #imports everything below
│   │   │   ├── Core.m         # extern "C" prototypes of the Rust API + thin ObjC wrapper
│   │   │   ├── EmuView.m      # NSView + CAMetalLayer + display link, keyboard/mouse events
│   │   │   ├── Renderer.m     # Metal pipelines, textures, CRT passes
│   │   │   ├── Audio.m        # AudioUnit (default output) render callback
│   │   │   ├── Keymap.m       # macOS kVK_* → Juku key table
│   │   │   ├── Disks.m        # disks in memory: open and save panels, the save alert, drag & drop
│   │   │   ├── Settings.m     # NSUserDefaults-backed settings
│   │   │   ├── Roms.m         # the system ROM table, loading ROM and expansion ROM files
│   │   │   ├── Snapshots.m    # snapshot files: meta, matching ROMs, the Quick Save file (§6.8)
│   │   │   ├── ShaderTypes.h  # CRTUniforms, shared by Renderer.m and Shaders.metal
│   │   │   ├── Shaders.metal  # decode + phosphor + CRT shaders
│   │   │   └── thumbnail/     # Quick Look thumbnail extension for snapshots, its own binary (§6.8)
│   │   │       ├── Thumbnail.m
│   │   │       ├── Info.plist
│   │   │       └── Thumbnail.entitlements
│   │   └── resources/
│   │       ├── Info.plist     # CFBundleExecutable=Juku, id io.github.sinijarv-juku.juku-emu, LSMinimumSystemVersion 14.0
│   │       ├── PkgInfo
│   │       ├── Juku.icon/     # app icon (Icon Composer): icon.json + Assets/label.svg, led.svg
│   │       └── en.lproj/, et.lproj/   # InfoPlist.strings
│   └── windows/               # Rust + Win32 + Direct3D 12 (§11)
│       ├── Cargo.toml
│       ├── build.rs           # shaders (d3dcompiler_47), version.h, rc.exe
│       ├── src/
│       │   ├── main.rs        # menus, commands, one instance, the main loop (≈ Main.m)
│       │   ├── audio.rs       # WASAPI output on a thread of its own (≈ Audio.m)
│       │   ├── disks.rs       # disks in memory, the save alert, the Insert and Save dialogs (≈ Disks.m)
│       │   ├── recent_disks.rs  # Recent Images and file references (≈ RecentDisks.m)
│       │   ├── settings.rs    # Juku.ini in %LOCALAPPDATA%\Juku (≈ Settings.m)
│       │   ├── emu_window.rs  # the window: pacing, drawing, keyboard (≈ EmuView.m)
│       │   ├── renderer.rs    # Direct3D 12 passes (≈ Renderer.m)
│       │   ├── Shaders.hlsl   # the passes of Shaders.metal in HLSL
│       │   ├── keymap.rs      # scan code → Juku key table (≈ Keymap.m)
│       │   ├── roms.rs        # the bundled ROMs and the user's, applying them (≈ Roms.m)
│       │   ├── settings_window.rs  # the Settings window (≈ SettingsWindow.m)
│       │   ├── keyboard_help.rs    # Input → Keyboard Mapping (≈ KeyboardHelp.m)
│       │   ├── snapshots.rs   # snapshot files, the sealed win section (≈ Snapshots.m)
│       │   ├── file_dialogs.rs     # the open and save dialogs
│       │   ├── file_types.rs  # Register File Types: HKCU\Software\Classes (≈ Info.plist's types)
│       │   ├── strings.rs     # resources/strings, parsed at startup (≈ Strings.m)
│       │   └── theme.rs       # light and dark mode, Settings → Appearance: title bars, menus, controls
│       ├── thumbnail/         # JukuThumbnail.dll, Explorer's thumbnails of snapshots (≈ src/thumbnail);
│       │                      # build.rs exports its COM entry points PRIVATE
│       ├── installer/         # Juku.iss, the installer, made with Inno Setup (§11.7), and its images
│       └── resources/         # Juku.rc (icon, version information), Juku.manifest,
│                              # Juku.ico and make_artwork.ps1, which makes it and the
│                              # installer's images from Juku.icon
└── resources/                 # shared by the front-ends
    ├── roms/ekta31.bin, ekta37.bin, jmon33.bin   # 16 KB system ROMs, copied into Contents/Resources/roms
    └── strings/en.strings, et.strings            # UI strings; Localizable.strings in the .app's en.lproj, et.lproj
```

**No headers within the Objective-C translation unit.** All Objective-C is compiled as one unit through `Main.m`, so its files `#import` each other directly and no function is declared twice. The rule does not cover a header whose job is to share definitions between separately compiled code, such as the Metal shaders and the Objective-C side.
- **Rust API.** The Rust FFI prototypes are written by hand as an `extern` block at the top of `Core.m`. Rust's `ffi.rs` is the source of truth, and a `// keep in sync with core/src/ffi.rs` comment marks the block. There is no cbindgen.
- **Shared structs.** Structs shared between Objective-C and Metal (the CRT uniforms) are defined once, in `ShaderTypes.h`, which `Renderer.m` imports and `Shaders.metal` includes. It uses the `simd` types, which have the same layout on both sides.

---

## 3. Build

### 3.1 The Cargo workspace
The root `Cargo.toml` is a workspace of the four crates: the core (`core`, crate `juku`), the debug server (`debugger/dap`, §12), the Windows app (`frontends/windows`) and its thumbnail handler (`frontends/windows/thumbnail`, §11). There is one `Cargo.lock`, the profiles are there only (cargo ignores a member's), and `.cargo/config.toml` puts cargo's output in `target/cargo`, so that `cargo clean` leaves the rest of `target/` (the app, the packages) alone. A relative `target-dir` there is taken from the repository's root, wherever cargo is run. The file holds nothing else: the build flags depend on `build.sh`'s and `build.cmd`'s modes, and in it they would apply to `cargo test` too. `default-members` leaves out the Windows crates, which build on Windows only, so `cargo test` and `cargo build` at the root take the core and the server. rust-analyzer does not go by `default-members`: it checks with `--workspace`, so on a Mac it reports errors from the Windows crates unless `rust-analyzer.check.workspace` is set to `false` in the editor.

```toml
cargo-features = ["panic-immediate-abort"]

[workspace]
members = ["core", "debugger/dap", "frontends/windows", "frontends/windows/thumbnail"]
default-members = ["core", "debugger/dap"]
resolver = "3"

[profile.release]
opt-level = 2
lto = true
codegen-units = 1
panic = "immediate-abort"

[profile.release.package.juku-dap]
opt-level = "s"                       # the server: small beats fast

[profile.dev]
opt-level = 0

[profile.dev.package.juku]
opt-level = 1                         # 8080 core must run at speed even in debug

[profile.test]
opt-level = 3                         # CPU exercisers run billions of cycles

[profile.test.package.juku]
opt-level = 3                         # the test profile inherits dev's override
```

`core/Cargo.toml` has no dependencies; its `[lib]` is `crate-type = ["staticlib", "rlib"]` (the static library for the app, the rlib for tests).

### 3.2 `build.sh`
It has three modes: default (release, tuned for this machine's CPU), `debug` (unoptimized, with debug info) and `dist` (release for other Macs).

1. **Core** (in the debug server's library, §12.2):
   ```sh
   RUSTFLAGS="-Zlocation-detail=none -Zfmt-debug=none" \
   cargo build -p juku-dap --target $RUST_TARGET \
       -Z build-std=std \
       --release      # omitted in debug mode
   ```
   - `--target` is required with `build-std`; `$RUST_TARGET` is the host's (`aarch64-apple-darwin` on Apple silicon).
   - Current nightlies treat `immediate-abort` as a real panic strategy taken from the profile. The old `-Z build-std-features=panic_immediate_abort` flag is now a hard error.
   - Output: `target/cargo/$RUST_TARGET/{release,debug}/libjuku_dap.a`.
   - Debug mode keeps `-Zfmt-debug` so `Debug` formatting works in logs.
2. **Objective-C:** one `clang` call on `frontends/macos/src/Main.m`, with the static library and the frameworks:
   ```sh
   clang $CFLAGS -Wl,-dead_strip frontends/macos/src/Main.m target/cargo/.../libjuku_dap.a \
       -fobjc-arc -fmodules -mmacosx-version-min=14.0 \
       -framework AppKit -framework AudioToolbox -framework Metal -framework QuartzCore \
       -o $APPDIR/MacOS/Juku
   ```
   - **Stripping.** Release and dist builds then run `strip` on the executable, which takes out its symbol table: 255 KB becomes 211 KB, a sixth less. `strip` keeps the UUID and renews the linker's ad-hoc signature; `package.sh` signs afterward. A copy with the symbols is left in `target/Juku.symbols`, so a crash report can still be read: `atos -o target/Juku.symbols -l <load address> <addresses>`. Debug builds keep their symbols.
3. **Shaders:** `_metal $APPDIR/Resources/default.metallib $MAC/src/Shaders.metal $MAC/src/ShaderTypes.h` (`$MAC` is `frontends/macos`), rebuilt only when the source or the header is newer.
4. **Info.plist:** `_plutil` converts it to binary form, only when the source is newer; then the version from `VERSION` goes in (§3.3).
5. **Resources:** hard-link `PkgInfo`, `resources/roms/*.bin` and the `.lproj` files into the bundle. The shared `resources/strings/<language>.strings` becomes `<language>.lproj/Localizable.strings`, next to the Mac's own `InfoPlist.strings` from `frontends/macos/resources`. Every build checks each link with `-ef` and re-links a file whose source was replaced (an editor that saves by renaming, `sed -i`, `git checkout`) or is new. A resource deleted from `resources/` stays in the bundle until `target/Juku.app` is removed.
6. **App icon:** `xcrun actool` compiles `frontends/macos/resources/Juku.icon` into `Contents/Resources/Assets.car` (the macOS 26 icon, named by `CFBundleIconName`) and `Juku.icns` (the fallback named by `CFBundleIconFile`). It runs only when a file in the icon bundle is newer than `Assets.car`. The paths are absolute, because actool passes them to a long-lived service that resolves relative paths against its own working directory. The icon is the Juku nameplate drawn white on black: a vertical gradient fill from `#1f1f1f` to `#0f0f0f` (the same as Activity Monitor), the JUKU letters in `#f0f0f0` with E5101 in gray (`label.svg`) and the red power LED (`led.svg`). Both layers have glass and shadows off, so it stays flat. The letters are SVG strokes, not text, because the SVG renderer does not lay out fonts. `Juku.icon` can be edited in Icon Composer.
7. **`target/`:** created by `build.sh` before anything else, and excluded from Time Machine with `tmutil addexclusion` when created.

8. **Quick Look extension:** `frontends/macos/src/thumbnail/Thumbnail.m` is compiled on its own (`-fapplication-extension -e _NSExtensionMain`, with Foundation, ImageIO and QuickLookThumbnailing) into `Contents/PlugIns/JukuThumbnail.appex`, stripped like the app, with its Info.plist converted and given the app's version. It is signed ad hoc with its sandbox entitlements (`Thumbnail.entitlements`), since macOS loads no unsigned extension; `package.sh` signs it with the real identity before it signs the app, because signing the app does not sign what is nested in it, and then verifies with `--deep`.

**Tests:** `cargo test` at the root runs the core's and the server's tests, without `build-std`; `cargo test -p juku` the core's only. The dev profile unwinds normally, so test failures report properly.

### 3.3 Version
The repository has one version, a semantic one (below): the file `VERSION` at its root, and nowhere else, so that a new version is a change to that one line. Everything built takes it from there:
- **macOS:** macOS takes numbers only, so `build.sh` writes `MAJOR.MINOR.PATCH` into the bundle's `CFBundleShortVersionString` and the build number into `CFBundleVersion`, in the app's and the Quick Look extension's Info.plist, whose sources have neither. The app shows `JukuVersion`, which `build.sh` writes too: the version, with a `-dev` build's commit (`0.5.0-dev+g046bcd2`), in the About panel and the snapshots' `meta` section. `package.sh` names the disk image with the version.
- **Windows:** `build.rs` writes `Juku.exe`'s version information: `FILEVERSION` is `MAJOR,MINOR,PATCH,` and the build number, and the strings are the version. The crate's `JUKU_VERSION`, which the About box and the snapshots show, is the version as on the Mac, with a `-dev` build's commit (§11.1). The installer reads the version from the exe, and `build.cmd dist` names the zip with it.
- **VS Code extension:** `package.py` writes it into the packed `package.json` and the `.vsix` manifest; VS Code wants semver too, and takes `0.5.0-dev`. The `package.json` in the repository says 0.0.0: VS Code refuses an extension with no version, and that file is what `--extensionDevelopmentPath` loads.
  - *As built:* the extension moved to juku-sdk (§12.8) and takes the SDK's version, by the same rules.
- **Crates:** `core`, `juku-dap`, the Windows app and its thumbnail handler give no version, so Cargo takes 0.0.0 and the lock files say so. They are not published, and nothing reads `CARGO_PKG_VERSION`.
- **Semantic versioning.** The form is `MAJOR.MINOR.PATCH`, numbers only, or that with `-dev` while the next version is being made: after release 0.4.0 the version is 0.5.0-dev until 0.5.0 is released. Semver puts 0.5.0-dev before 0.5.0, and so does the build number, which is 0 for a `-dev` version and 1 for a release: 0.5.0 build 0 comes before 0.5.0 build 1, on macOS and in Windows's `FILEVERSION` (0,5,0,0 before 0,5,0,1). `build.sh`, `build.rs` and `package.py` refuse a version in another form.
  - **0.x** (now) is initial development, and promises nothing: any release may break compatibility with earlier ones, such as dropping an older snapshot format. A release that adds or changes something bumps MINOR, one with fixes only PATCH.
  - **From 1.0.0**, what a release keeps working is its interface: the files it reads and writes (disk images, snapshots, ROM and expansion ROM files), its settings and command line, and the debug server's protocol, launch and attach settings and the debug information it reads.
    - **PATCH:** fixes only.
    - **MINOR:** features that keep that interface: everything an earlier version wrote is still read, every setting still means what it did.
    - **MAJOR:** a release that does not: a snapshot or a setting no longer read, a launch setting changed.
  - **A release** is the commit that drops `-dev` from the version, `build: bump the version to X.Y.Z`, tagged `vX.Y.Z`. The next commit starts the next version, `build: start version X.Y+1.0-dev`.
  - *As built:* version 0.4 became 0.4.0, the version moved from Info.plist's `CFBundleShortVersionString` to `VERSION`, and development went on as 0.5.0-dev.

Format versions are another matter and change with their formats: the snapshot format (§4.6), the sealed host sections (§6.8) and the debug information (§12.6).

### 3.4 `package.sh`
Builds with `build.sh dist`, signs the Quick Look extension and then the app with `CODE_SIGN_IDENTITY`, makes `target/Juku-<version>.dmg`, signs it and, with `NOTARY_PROFILE`, notarizes it and staples the ticket. Releases are signed on the developer's Mac, not in CI, so that the Developer ID certificate stays in its keychain.

The disk image opens as a window like other apps' installers: 660 × 400 points, no toolbar or sidebar, the app on the left and a link to `/Applications` on the right, both at 160 points, on a light background with a chevron between them. The background stays light in dark mode, where Finder still writes the labels dark on it; Finder's own background would follow the mode, but a picture replaces it, and Finder fills a picture's transparent parts with white, so the chevron needs the picture. The volume's icon is the system's white removable-disk icon with the app's on it.
- **Artwork:** `frontends/macos/dmg/DiskImageArt.m`, a small AppKit program that `package.sh` compiles and runs, draws the background at 1× and 2× (`tiffutil -cathidpicheck` joins them in `.background/background.tiff`) and the volume icon in every iconset size, from `IOStorageFamily.kext`'s `Removable.icns` and the build's `Juku.icns`, which `iconutil` packs as `.VolumeIcon.icns`; `SetFile -a C` tells Finder the volume has an icon of its own. Made, not drawn, so the icon follows the app's.
- **Layout:** Finder keeps a window's layout in the volume's `.DS_Store`, with an alias to the background picture, and only Finder writes one that it reads back reliably. So the image is made writable first (`UDRW`), mounted, and laid out by Finder through AppleScript: the view, the bounds, the icon size, the background and the two positions; then the volume icon goes in, after Finder's work, since Finder's `update` deletes a `.VolumeIcon.icns` it finds; then the image is converted to the compressed, read-only `ULFO` image that is signed. The first run asks to let the terminal control Finder (Privacy & Security → Automation). A volume of the same name must not be mounted, since Finder finds the disk by its name.
- **The `.dmg` file's own icon** stays the system's: a custom icon on a file lives in its resource fork, which a download does not keep. What carries the icon is the volume, in the window's title, the sidebar and on the desktop.

---

## 4. Core architecture (Rust)

### 4.1 Time base and scheduling
- **Master time** is the CPU cycle counter `cycles: u64`, at 2 MHz.
- **Derived clocks:**

| Clock | Rate | In CPU cycles |
|---|---|---|
| PIT #1 (all channels), video byte clock | 1 MHz | ÷ 2 |
| PIT #3 ch1 (beeper) | 2 MHz | ÷ 1 |
| PIT #3 ch0 (baud), and ch2 when set so | 16 MHz / 13 = 1.2308 MHz | fractional accumulator, ×8/13 |
| FDC | 1 MHz | ÷ 2 |
| Scanline | 64 µs | 128 cycles |
| Frame (313 lines) | ≈ 49.92 Hz | 40 064 cycles |

- **Run loop.** `Machine::run_cycles(n)` adds `n` to a target time and steps until emulated time reaches it. Instructions are never split, so a call can overrun by a few cycles; the overrun is deducted from the next call. Each step:
  1. Execute one instruction (or service a pending interrupt acknowledge).
  2. Advance the devices by the cycles consumed.
  3. Re-sample the 8259 INT line.
- **Device advance:**
  - **PITs.** The behavior is that of clocking every counter pulse by pulse: PIT #1 at 1 MHz, PIT #3 ch1 at 2 MHz and ch0 at 8 pulses per 13 cycles. Counters clocked from another OUT get their pulse on its falling edge. This keeps the cascaded gate and clock wiring simple and exact.
  - **As built: PIT pulses are batched.** Most pulses only count down, so the counters on fixed clocks are not clocked one pulse at a time.
    - `Pit::quiet(ch)` gives the number of coming pulses that change neither OUT nor any state but the count, and `Pit::skip(ch, n)` applies that many in one step.
    - The machine keeps the pulse number of the next eventful pulse for PIT #1 (all three channels together) and for PIT #3 ch1. Only that pulse goes through `Pit::clock`, with its OUT changes passed on as before. RomBios video timing gives 4 such pulses per scanline instead of 64.
    - `System::next_event` is the cycle of the earliest of these and of the mouse poll. `advance` does nothing but one comparison until then.
    - A PIT is brought up to date (`sync_pit`) before its ports are read or written, and the events are worked out again after a write.
    - PIT #3 ch0 (baud rate) drives nothing, so it has no events; nor does ch2 on its 1.2308 MHz clock. It is brought up to date on a PIT #3 access and at the end of `run_cycles`, with `Pit::run`, which leaves out whole periods of a free-running counter.
    - A counter in BCD mode holding digits above 9 does not count evenly and falls back to one pulse at a time.
    - At the monitor prompt the core ran about 175× real time in an optimized build, and about 40× before batching. The raster (Video, §4.5) brought it to about 160× (see Speed below).
  - The FDC and the mouse are **event-driven**: each keeps `next_event_cycle` and is only stepped when that time is reached.
  - **As built:** the FDC is advanced lazily, only when a port 1C–1F access or a port C write reaches it. Disk rotation is computed from absolute time (index pulse when `t mod 400 000 < 8 000`), and INTRQ is not wired, so nothing needs to run between accesses.
- **Accesses inside an instruction.** Devices are advanced after each instruction, but a bus access is timed where it happens in it, so that a write to a timer, to POF or to video memory lands on the right µs of the raster:
  - An I/O read or write (`IN`, `OUT`) happens 8 T-states into the instruction, in T2 of its third machine cycle (`IO_AT`). `input` and `output` first bring every device up to that time (`catch_up`, which runs the events due by then), then handle the port with `cycles` set to it, then put `cycles` back. A stall on the FDC data register is counted from that time too.
  - A memory write happens in T2 of the machine cycle that writes (`write_at`): 5 T-states in for `MOV M,r` and `STAX`, 8 for `MVI M`, `INR M` and `DCR M`, 11 for `STA`, `SHLD` and `XTHL`, 6 for `PUSH` and `RST`, 12 for `CALL` and an interrupt's CALL, and a second write 3 T-states after the first. The CPU tells the bus each instruction's opcode (`Bus::begin`), and the bus counts its writes. Only a write that may race the beam needs the time (Video, §4.5).
- **Speed.** `cargo run -p juku --profile test --example bench` times four workloads, best of three:

| Workload | Before the raster | With it |
|---|---|---|
| Idle `*` prompt | 176× | 160× |
| Monitor dump scrolling the screen | 163× | 153× |
| `demos/bounce.bin` (one timer write per frame) | 175× | 165× |
| `demos/scroller.bin` (the CPU shifting video memory, half of each frame) | 165× | 149× |

  - **Code placement moves these figures.** In the test profile a change that leaves the emulation's code alone can still move it in the binary: adding the debugger's loop (§12.4) took `bounce.bin` from 178× to 139× while the other three stayed within 2%. With functions and blocks aligned (`-C llvm-args=-align-all-functions=6 -C llvm-args=-align-all-nofallthru-blocks=5`) both builds gave 177–179×, so it was where the loop's code fell, not more work.
  - **To compare before and after, build as the app is built:** the release profile, with LTO, through `build-std`: `cargo run -p juku --release -Z build-std=std --target $(rustc -vV | sed -n 's/^host: //p') --example bench`. Before the debugger this gave 172×, 176×, 202× and 175× (M1 Max, `-C target-cpu=native`), and the same within 1% with the debugger's code in.

- **CPU stalls.** When the FDC data register is accessed during a Type II/III command while BUSY = 1, DRQ = 0 and PA6 = 0 (MD §10.3):
  - `Machine::io_read`/`io_write` advance time to the FDC's next DRQ or INTRQ event, adding the stall to the instruction's cycle count, and then complete the access.
  - This is the equivalent of the board's READY/WAIT logic, and of MAME's HALT-and-rewind.
  - A safety timeout of six disk revolutions (1.2 s) ends the stall: a read returns the data register contents and a write is dropped. It must be longer than any wait a command can make: a Read Sector given just after its sector's ID went by gets its first byte more than a revolution later, and a sector that is not there ends the command after five revolutions and the head settling. The timeout was one revolution at first; QDISK.COM's random seek test then read a sector one byte late, the stale data register first, now and then.

### 4.2 CPU — `cpu/i8080.rs`
- **Registers:** A, F, B, C, D, E, H, L, SP, PC, plus the INTE and HALT states.
- **Flag rules:** S, Z, AC, P, CY; bit 1 = 1, bits 3 and 5 = 0. Follow the 8080 rules, not the Z80's:
  - ANA sets AC = (op1 | op2) bit 3.
  - SUB, SBB and CMP set AC from the borrow-inverted rule.
  - DAA as on the 8080.
- **Undocumented opcodes:** all 256 are implemented with their 8080 aliases.
  - 08/10/18/20/28/30/38 = NOP
  - CB = JMP
  - D9 = RET
  - DD/ED/FD = CALL
- **Cycle counts:** taken from a 256-entry table. Conditional CALL and RET add their extra cycles when taken (11/17 and 5/11).
- **Bus trait:**
  ```rust
  trait Bus { fn read(&mut self, a: u16) -> u8; fn write(&mut self, a: u16, v: u8);
              fn input(&mut self, port: u8) -> u8; fn output(&mut self, port: u8, v: u8);
              fn int_pending(&self) -> bool; fn int_ack(&mut self) -> [u8; 3]; }
  ```
- **I/O stalls** (for example, the FDC wait) are not part of the CPU interface. The machine's bus records stall cycles during an access and adds them after `Cpu::step` returns.
- **Interrupts:**
  - When INTE = 1 and INT is asserted, the CPU performs INTA.
  - The 8259 supplies `CD lo hi` (a 3-byte CALL, 8080 mode). The CPU executes the CALL and charges 17 cycles plus the INTA overhead.
  - EI takes effect after the following instruction.
  - HALT wakes on an interrupt.
- **Validation:**
  - `core/tests/cpu_exercisers.rs` runs TST8080, 8080PRE, SuperSoft CPUTEST and 8080EXM under a tiny CP/M shim. The shim handles BDOS 2 and 9 by trapping 0005h. As in the superzazu/8080 harness, it places `OUT 0` at 0000h and `OUT 1 / RET` at 0005h, so its totals compare one-to-one with that harness's published figures.
  - All four pass, and their instruction and cycle totals match exactly:

| Program | Instructions | Cycles |
|---|---|---|
| TST8080 | 651 | 4 924 |
| 8080PRE | 1 061 | 7 817 |
| CPUTEST | 33 971 311 | 255 653 383 |
| 8080EXM | 2 919 050 698 | 23 803 381 171 |

  - The cycle table matches MAME's `lut_cycles_8080`; for example, XCHG takes 4 cycles.
  - 8080EXM takes about 19 s and is `#[ignore]`d. Run it with `cargo test -p juku --test cpu_exercisers -- --ignored`.
  - The binaries come from github.com/superzazu/8080 (its `cpu_tests`, from altairclone.com). They were once kept in `core/tests/data/`.
  - *As built:* they are other people's programs, so they are not in the repository. Each test downloads its program with `curl` (part of macOS, and of Windows 10 and later) from superzazu/8080 at a fixed commit, so the file cannot change, into `target/cpu_tests/`, and reads it from there afterward (outside `target/cargo`, so that `cargo clean` keeps them); a file of the wrong size is an error that says to delete it. A test whose program cannot be downloaded is skipped with a note, and reports as passed.
  - `[profile.test]` uses opt-level 3, which gives about 150 M instructions/s.

### 4.3 Memory — `machine.rs`
- **Storage:**
  - `ram: [u8; 65536]`
  - `rom: [u8; 16384]`, loaded from FFI, both EPROMs in one image
  - The external window: 32 KB holding the expansion ROM from its start, FFh after it and when none is plugged in
  - `ext_pages: Option<Vec<[u8; 32768]>>`, a later phase
- **Mode** = PPI #1 PC1..0, re-evaluated whenever port C is written (MD §3.2):

| Mode | Reads | Writes |
|---|---|---|
| 0 | 0000–3FFF from ROM | all to RAM |
| 1 | D800–FFFF from ROM\[1800 + (a − D800)\] | all to RAM |
| 2 | same as mode 1; 4000–BFFF from the external window | 4000–BFFF dropped; others to RAM |
| 3 | all RAM | all to RAM |

- **Read path:** a 64-entry read map (1 KB granularity) holding pointers to slices, rebuilt on each mode change. This keeps the hot path to a table lookup.
- **Key click patch.** `Machine::set_key_click(false)` silences the RomBios key click by patching the ROM in place: the click's `JMP 0597h` becomes `RET`, and the ROM test's expected sum for that chip, at ROM 0008h, is adjusted to match (MD §11, §14.6). `true` restores both bytes. The 20 bytes of the click code are looked for in the chip at ROM 1000–17FF, with the JMP or the RET: RomBios 3.43m and 3.43 #0031 have the JMP at 1270h, 3.43 #0035 at 1276h. The patch is made only if the code is there exactly once and the chip's sum equals the byte at 0008h, the layout all three bundled ROMs share. Otherwise nothing changes and the call returns false, so a ROM file of another layout is left alone. The patch survives resets, because reset does not reload the ROM. It works whatever the guest's CONCH bit 7, and software cannot turn the click back on.
- **ROM swap.** `Machine::load_rom` copies a new 16 KB image over the ROM and does a cold reset, like changing the EPROMs with the power off. Disks stay in their drives. A wrong size changes nothing and returns false. The key click patch goes with the old image, so the host applies it again.
- **Expansion ROM.** `Machine::insert_expansion` plugs in the plug-in memory expander's ROM (MD §3.2) and `eject_expansion` takes it out. Both cold-start the machine: the real expander sits on the I-41 system bus and is changed with the power off. Disks stay in their drives.
  - **Check.** `expansion::check` accepts an image of 1 byte to 32 KB that starts with the bootable program header the monitor's `A` command loads (MD §14.1): `JMP` to an address in the program as copied to 0100h (from 0103h on, since the header may run as code), a key word, and a length of at least 7 that fits in the image. Anything else is refused and nothing changes. JBASIC 1.1 passes: `JMP 0107h`, key DCBAh, length 2000h.
  - **Pages.** The page number the RomBios puts on PA0–PA2 with the PA5 strobe is ignored, as in MAME: the expansion ROM is in every page. External RAM pages and a RAM disk are not emulated.
- **Reset:**
  - **Cold** fills RAM with 00h; the ROM's own test then runs.
  - **Warm** (RESET button) keeps RAM. That gives the BIOS warm-start behavior through its D75C signature check.

### 4.4 I/O decode
- Decode on the low byte only: the board decodes A8–A15, which carries the same value during IN/OUT. Ports 00–1F are on-board; others go to the bus.

| Port | Device |
|---|---|
| 00–03 | 8259 (A0) |
| 04–07 | PPI #1 |
| 08–0B | 8251 stub |
| 0C–0F | PPI #2 |
| 10–13 / 14–17 / 18–1B | PIT #1 / #2 / #3 |
| 1C–1F | 1793 |
| 80–81 | mouse card |
| other | read FFh, write ignored; log once per port in debug builds |

### 4.5 Devices
- **8259 (`pic8259`).**
  - Supports ICW1–ICW4 (no ICW4 in 8080 mode), OCW1 (mask), OCW2 (non-specific and specific EOI, rotate), OCW3 (read IRR/ISR, poll), edge and level triggering, fully-nested priority, and the 8080 CALL vector with interval 4/8.
  - IR inputs: IR2/IR3 = 8251 RxRDY/TxRDY (inactive in v1), IR5 = inverted PIT #2 OUT1 (frame), IR6 = mouse.
- **8253 ×3 (`pit8253`).**
  - All six modes; binary and **BCD** counting; control word, latch, and the LSB/MSB/word read-write sequences; gate behavior per mode; the OUT pin with change callbacks.
  - Clock and gate wiring (MD §7.2, cross-checked with MAME):

| Signal | Connects to |
|---|---|
| PIT1 ch0..2 CLK | 1 MHz |
| PIT1 OUT0 | PIT2 CLK0; GATE of PIT1 ch1 and ch2 |
| PIT1 OUT1 | HOR RTR: the raster |
| PIT1 OUT2 | its rising edge fires the horizontal sync, which starts a raster line and clocks PIT2 ch1 and ch2 |
| PIT2 OUT0 | GATE of PIT2 ch1 and ch2 |
| PIT2 OUT1 | VER RTR: the raster, 8259 IR5 (inverted), and PIT3 CLK2 by default |
| PIT2 OUT2 | its rising edge fires the vertical sync: the frame is published |
| PIT3 CLK0 | 1.2308 MHz |
| PIT3 CLK2 | VER RTR, or 1.2308 MHz with `set_t32_fast_clock(true)` |
| PIT3 CLK1 | 2 MHz |
| PIT3 OUT1 | speaker |

  - Clock inputs that come from another counter's OUT are delivered as edges, through the OUT-change callback.
  - **T3.2's clock is an option** (`Machine::set_t32_fast_clock`, `docs/T3.2_and_TIMCNT.md`). By default VER RTR clocks it once a frame, as in MAME. The RomBios stopwatch TIMCNT, which adds T3.2's pulses at every frame interrupt, then counts in the 20 ms units the manual gives, which programs that time themselves with it (PUSHER, LOGER, LAOLEO and others) expect. With the option on, it counts at 1.2308 MHz with the baud rate counter, as the 1987 schematic draws it. RDEM then gets random numbers from it, and TIMCNT adds about 24 600 a frame. The switch brings PIT #3 up to date first, so the pulses before it come from the old clock. It survives resets and snapshot loads: it belongs to the host, which applies its setting.
  - The sync pulses come from 74123 one-shots fired by rising edges (MD §7.2). PIT2 ch1 and ch2 are clocked by the horizontal one-shot's Q̄, which falls when it fires: on the rising edge of PIT1 OUT2, 8 µs into the line with the RomBios counts. Until the raster they were clocked on its falling edge, at the start of the line, which moved VER RTR and IR5 by those 8 µs and made no other difference.
- **8255 ×2 (`ppi8255`).**
  - Mode 0 is required. Modes 1 and 2 can be minimal, since neither is used.
  - Port C bit set/reset is supported.
  - PPI #1 callbacks:
    - **PA** → keyboard column, PA4 volume, PA5 page strobe (logged), PA6 FDC wait-disable.
    - **PB** ← keyboard (MD §6.4).
    - **PC** → memory mode, floppy motor, density, drive select, side, 8" (ignored), POF (picture off).
  - PPI #2 is unconnected: port reads return FFh, so the printer always looks busy.
- **8251 (`usart8251`).** A stub. It accepts mode and command words; status reports TxRDY = 1, TxE = 1 and RxRDY = 0; transmitted bytes are discarded. That is enough for the BIOS "NetBios" init not to hang. The network menu choice will just fail.
- **Keyboard (`keyboard`).**
  - State: `keys: [u8; 16]` (bit r = row r pressed), `shift`, `ctrl`, and the DIP banks `dip_pb5: u8` and `dip_pb4: u8`.
  - Port B is computed on demand from the current PA column (MD §6.4): 74148 priority, the highest row wins, active low.
  - Defaults: PB5 bank all OFF (reads 1), PB4 bank reads 0, as in MAME.
  - The API takes **Juku key IDs** (`col << 3 | row`, plus SHIFT = 0x80 and CTRL = 0x81). Host keycode mapping stays in Objective-C.
  - **Caps Lock follower (`CapsFollower`).** The CAPS key only toggles, and RomBios keeps the state in CONSW (D4BAh) bit 0 (MD §14). `Machine::set_caps_lock` sets a target, the host's lock state, and the core makes the guest follow it:
    - At the start of each vertical blank (VER RTR falling), if CONSW bit 0 differs from the target and software read the CAPS column (PA = 3) during the last frame, the core holds CAPS for 3 frames, then waits 5 frames before it looks again.
    - The scan condition keeps presses out of the boot window. Until RomBios unmasks IR5 (about 1.5 s into a cold boot), nothing reads the keyboard, and CONSW holds RAM test patterns.
    - Each sync request allows 4 presses. A new target or a reset (warm or cold, because startup clears CONSW) starts a new request. A program that keeps its own keyboard state and leaves CONSW alone gets at most 4 stray CAPS presses, not a stream of them.
    - With no target set (tests that never call it), the follower does nothing.
- **FDC 1793 (`fdc1793`).**
  - Commands: Restore, Seek, Step / Step-in / Step-out (with and without track update), Read Sector, Write Sector (single and multi), Read Address, Read Track, Write Track, Force Interrupt (D0, D8 and the index variants).
  - Status: Type I (index pulse, TR00, head loaded = 1, seek error, CRC, write protect, NOT READY) and Type II/III (DRQ, lost data, CRC, RNF, record type, write fault = 0, NOT READY).
  - Clocked at 1 MHz, so the step-rate codes 0–3 give **6/12/20/30 ms** (the datasheet's 1 MHz column), and the V/E settle time is 30 ms.
  - **Byte timing:** one byte every 32 µs (64 cycles) in MFM and 64 µs in FM. DRQ is raised per byte. Lost-data rules apply when the CPU is late, but cannot normally trigger, because the CPU stalls.
  - **Write Track** (formatting): the written stream is parsed for ID address marks (F5/F6/F7/FE/FB handling as in the datasheet). Each ID field gives (C, H, R, N), and the data field that follows is written into the image when the geometry matches. A mismatched geometry is reported once as "unsupported format" and otherwise ignored.
  - **Track layout:** as on real disks, which the Juku's formatters wrote (MD §10.3), not MAME's `.juk` gaps: 32 × 4E, the index mark, 32 × 4E, then the sectors in the order 1–10, 601 bytes each (gap 2 = 22, gap 3 = 24 × 4E and three A1), then 4E up to byte 6181, where the BIOS's Write Track stops, and 00 to the end of the 6250-byte revolution. In the emulator EKDOS, which reads every other sector, misses each next one and waits a revolution for nearly every sector: gtr.com (27 KB) loads in 12.5 s, where the old skewed layout took 5.2 s. That the real machine loaded this slowly is an assumption, not tested on hardware (MD §10.3).
  - **Read Track:** returns that track, built from the image's sectors.
  - **Side compare:** honored (S flag, C bit).
  - **Not modeled:** HLT is tied true and TG43 is unused (MD §10.3).
- **Drives (`floppy`).**
  - Two `Drive`s, each holding: an `Option<Disk>`, the head position (0–83), the motor flag, a rotation phase (300 rpm → 200 ms per revolution, index pulse 4 ms wide), and a write-protect flag.
  - **Selection:** PC5 picks the drive. PC2 = motor, which is shared but only meaningful for the selected drive (MD §10.3). PC6 = side.
  - **Ready:** READY = disk present AND motor on AND at least 2 revolutions since motor-on. This status feeds the FDC's NOT READY bit.
- **Disk images (`floppy::Disk`).**
  - The MAME `.juk` format is raw sector data with no header (MAME `juku_dsk.cpp`):

| Size | Geometry |
|---|---|
| 819 200 bytes | 80 cylinders × 2 heads × 10 sectors × 512 bytes, MFM, sector IDs 1–10 |
| 409 600 bytes | 80 cylinders × 1 head × 10 sectors × 512 bytes, MFM |

  - Layout is track-major with the heads interleaved: `offset = ((cyl * heads + head) * 10 + (id − 1)) * 512`.
  - Other sizes are rejected with an error.
  - `.img`, `.dsk` and `.cpm` files of the same sizes are accepted as aliases. The test images `jukgame1.juk` (games) and `jukprog1.juk` (programs) are 819 200-byte double-sided images. MAME's other containers (HFE, MFM) are out of scope.
  - **Blank disks.** `Disk::blank` (`Machine::insert_blank_disk`) is a double-sided image with every byte E5h (`FORMAT_FILL`), what formatting leaves in a sector: EKDOS, like CP/M, takes such a directory as empty, lists no files on it and saves to it. Its system tracks are E5h too, so it does not boot.
  - The interleave in the BIOS table (MD §10.3) is a logical-to-physical mapping done by software. Physically, sectors are read by ID, so no interleave is stored in the image.
  - **The disk in memory.** A drive's disk is the copy the machine writes to; the core never touches files. A `dirty` flag, the disk's **changed** flag, is set by any write to it, even of the bytes a sector already held, and cleared by the host after a save (`Disk::clear_dirty`). A disk starts clean when it is put in and when a snapshot restores it (§6.3).
- **Mouse (`mouse`)** (MD §10.4, MAME `jukumouse.cpp`):
  - Inputs: accumulated host deltas `dx`, `dy` (in Juku counts) and the button bits.
  - A 600 Hz poll event raises IR6 when `dx ≠ 0`, `dy ≠ 0` or the buttons changed.
  - `IN 80h` returns `0x80 | stepY<<5 | dirY<<4 | stepX<<3 | dirX<<2 | left<<1 | right`, then takes one count off each non-zero axis.
  - Direction bits: X right = dir 0, X left = dir 1; Y down = dir 1, Y up = dir 0.
  - Port 81h mirrors 80h.
  - **As built:**
    - The backlog is capped at ±64 counts per axis.
    - A poll that finds the previous event still unread drops one count per axis. The card's latches hold one step, so a slow reader sees less movement rather than lag.
    - A read lowers IR6. A poll with something to report makes a new rising edge.
    - `juku_mouse_enabled` reports whether IR6 is unmasked.
  - **Deliberate differences from the traced card** (MD §10.4):
    - A button release raises IR6, as in MAME. The card's button latches clock only on a press, so a release causes an interrupt only through contact bounce. Without one, gtr.com would not see the release until the next movement, and a drag could keep drawing.
    - The direction bits are set only together with a step and read 0 otherwise. On the card they always show the last direction. Software reads a direction bit only with its step bit, and a fixed idle value of 80h keeps reads predictable.
    - Bits 7 and 6 read 1 and 0, as in MAME. The card does not drive them, so on the real machine they read whatever an undriven bus line gives, which is not known.
    - A warm reset leaves the mouse state alone. It is not known whether the RESET button pulls bus −INIT, which would clear the card's latches. It makes no practical difference: the BIOS masks IR6 again, and gtr.com reads port 80h once when it hooks the interrupt.
    - **Tests:** `core/tests/mouse.rs` runs gtr.com from the programs disk, `jukprog1.juk`. It answers "Have you mouse attached" with Y and presses a key past the help screen. It then checks that 40/30 counts move the cursor exactly 40/30 pixels, and that dragging with the left button draws a 50-pixel line. gtr reads up to 600 events/s.
- **Video (`video`).** The raster as the beam draws it, from the timer outputs, as the schematic wires them (MD §7.1, §7.2, §7.5). `Raster` keeps the state; `System` feeds it the events.
  - **The hardware it follows:**
    - The video address counter steps once per µs while HOR RTR (PIT1 OUT1) is high, whatever VER RTR and POF do, and is loaded with D800h on the rising edge of VER RTR (PIT2 OUT1).
    - A byte is shown while HOR RTR and VER RTR are both high. POF (PC7) blanks the output and nothing else.
    - The video base address is fixed at D800h: S3 switches are not exposed, though there is a constant for them.
  - **The beam.** The monitor places the picture by its sync pulses, so the raster does too: x is the time since the start of horizontal sync (the rising edge of PIT1 OUT2), y the number of lines since the start of vertical sync (the rising edge of PIT2 OUT2). The raster is 64 µs × 313 lines from there, one byte per µs (`RASTER_BYTES` × `RASTER_LINES`). Bytes shown past it are dropped.
    - Without sync the monitor's oscillators run on: a line starts 72 µs after the last one, and a frame after 340 lines. Only a program that stops the sync timers sees this.
    - The monitor follows sync at once; a real one would follow a jump in the line rate with some delay (MD §13).
  - **Drawing lazily.** Time is counted in µs, the pulses of PIT #1. Between events the state is constant, so the bytes shown since the last event are copied from RAM in one run when the next one comes (`Raster::advance`). The events:
    - HOR RTR, VER RTR and POF changing, and the two sync pulses (`set_hor`, `set_ver`, `set_pof`, `hsync`, `vsync`);
    - every port access, which brings the raster up to its time (§4.1);
    - a CPU write that may race the beam, below.
    - With the RomBios timing that is 3 runs a line, as many as the timer events PIT #1 already had.
  - **Writes racing the beam.** The bytes the beam fetched since the last event are not copied yet, so a write to one of them must draw up to the write first, with the old value. While bytes are shown, the raster keeps a watch range of 64 addresses from the counter (`watch`, `watch_len`; empty while nothing is shown). `Bus::write` compares the RAM address against it, two comparisons, and on a hit draws up to the write's time inside the instruction (`racing_write`, §4.1). Writes ahead of the beam in the watch range draw early, which costs time and changes nothing.
  - **Publishing.** At the start of vertical sync the frame is complete: it becomes the front frame, `seq` counts up, `frame_cycle` is set, and the other buffer is cleared for the next frame. `Geometry` is worked out from the PIT counts then, BCD-decoded (MD §7.2), and published as the frame's active area:
    - bytes per line = C(10h) − C(11h)
    - visible lines = C(14h) − C(15h)
    - first byte = C(11h) − C(12h) µs after the start of horizontal sync; first line = C(15h) − C(16h) lines after the start of vertical sync
    - Defaults: 40 bytes × 241 lines (320×241), from byte 16 and line 47. The 384×200 mode works automatically.
    - The active area is where a program that keeps the timing draws. The renderers ignore it; the tests read text through it (`Frame::active_line`).
  - **Polarity.** PIT2 OUT1 (mode 1) is low for the 72 blank lines and high for the 241 active ones. The schematic inverts it onto IR5 (MD §7.3), so its **falling edge** is the start of vertical blank and the IR5 request edge. MAME wires OUT1 to IR5 without the inverter, which moves the interrupt to the end of blank; the period is the same.
  - **History.** The first version copied the active area from RAM once a frame, at the start of vertical blank. That missed every timing change within a frame, and caught a program that writes video memory across the frame boundary half done: `demos/scroller.bin` showed its text a pixel apart in 159 of 600 frames. By the program's timing, the real machine would draw that text long after the program has finished writing it.
  - **Published frame:**
    ```rust
    #[repr(C)] pub struct Frame { pub seq: u64, pub active_x: u16, pub active_y: u16,
        pub active_w: u16, pub active_h: u16, pub _pad: [u8; 8], pub data: [u8; 64 * 313] }
    ```
    It is double-buffered inside the machine. `seq` increments per frame, so the renderer only uploads when it changes.
- **Audio (`audio`).**
  - The speaker level is a step function of time: `OUT1 ? (PA4 ? 1.0 : 0.5) : 0`.
  - Every level change is recorded with its cycle timestamp.
  - At 48 kHz (41.667 cycles per sample) each output sample is the box-filtered average of the level over its period.
  - The result then goes through a 1-pole DC-blocking high-pass (~20 Hz) and a gentle 1-pole low-pass (~8 kHz) to imitate the small speaker.
  - Samples go into an **SPSC lock-free ring** (`AtomicUsize` head and tail, 8192 stereo frames, each a left and a right f32 in one `AtomicU64`). The speaker is the same in both channels; the drive sounds are panned (below). The ring is a **separate heap object** from the machine, so the audio thread never aliases `&mut Machine`.
- **Drive sounds (`drive_sound`).** The floppy drives can be heard: the spindle motor, the head stepping, and a disk going in and out with the door lever. `Machine::set_drive_sounds` turns them on or off; they are off in a new machine, and the hosts turn them on from their settings (§6.7). They are mixed into each sample after the speaker's filters, since the drives are not heard through the speaker.
  - **Made from noise, after recordings.** The sounds follow recordings of a real 5.25" drive: the lever closing, an eject, a disk going in, a disk spinning, single steps and a 40-track seek. They are synthesized as they play, with no sample data, and the recordings are not in the repository. One white noise goes through 21 third-octave band-passes (125 Hz to 12.5 kHz, each two RBJ stages of Q 2.9, scaled to unit RMS for white noise), and each sound is a level per band over time. The sounds share the noise: each band's level is the square root of the sounds' powers summed, which is what noises of their own would give.
    - **A knock** rises linearly over its attack (1–17 ms) to a level per band and dies away with a time constant per band. A loud one also pushes the drive frame's six resonances (193, 392, 616, 1216, 1448 and 2266 Hz, ringing 60 to 8 ms), which the recordings show as narrow peaks shared by the lever, the eject and the seek. Past its peak a knock joins a tail of its kind, whose power per band decays as one, so a fast seek costs no more than a single step.
    - **A rub**, a disk sliding, is a level per band under an envelope given every 10 or 20 ms in dB, made uneven by a random wander of about 2 dB.
    - **The numbers** are in `drive_sound/fitted.rs`, fitted with a script outside the repository. Each event of a recording was cut out and measured per band, less the background: a knock's attack, its peak per band, a decay per band (smoothed across bands) and the resonances standing out above their neighborhood; a rub's mean per band and envelope. The synthesis, rendered the same way, was then adjusted until each recording's band energies matched: each event alone, then each recording whole, since a knock's tail runs into the next, and the resonances to the recordings' peaks. The rendered sounds come within about 1 dB of the recordings in level (RMS), the seek 1 dB under, and mostly within 3 dB per band.
  - **The motor.** PC2 is the shared motor line, so both drives spin while it is on (MD §10.3). Each motor hums: eight tones from 236 to 994 Hz, strongest at 288 and 296 Hz, which rise with the speed. A drive with a disk clamped adds the disk's steady noise (strongest around 315 Hz and 2.5 and 8 kHz) and, once a turn, a short swish around 8 kHz; an empty drive turns with a fifth of that noise. The recorded drive turned at 360 rpm, but the swish keeps the emulated drive's 300 rpm. The speed rises with a time constant of 0.08 s and falls with one of 0.1 s, as fast as the recorded motor starts and stops. RomBios turns the line on with the first disk access and never off (MD §10.3), so from the first `T` the drives spin until the next reset; at the monitor prompt after power-on they are still.
  - **Stepping.** Each step pulse to the selected drive is the recorded step: about 50 ms of noise peaking at 2 kHz, rising over 17 ms. Steps less than 15 ms apart are a seek, where the recorded steps are at half the level and every other one at 0.7 of that; RomBios steps every 20 ms, so its steps are heard whole. A step against the head's stop is 1.4 times as loud and rings the frame twice as hard.
  - **A disk going in:** a knock as the disk meets the slot, 0.56 s of sliding, a last scrape and two clicks as it settles (0.8 s). After a pause of 0.25 s the lever turns down: three ticks, the cam rubbing, a tick and the latch, 0.43 s after the lever starts, when the disk is clamped and its noise starts. **Going out:** the lever flips up with a clack in three parts and lets the disk go, and 0.2 s later the spring pushes the disk out. A disk put in where there is one plays both, the new one 0.6 s after the old one starts out.
  - **Timing.** The FDC records each motor change, step and command with its cycle in `Fdc::sounds` (at most 64). Before the speaker is brought up to a cycle, the machine advances the FDC to it and hands the speaker what is recorded (`System::drive_sounds`): on every level change while the sounds are on, and at the end of `run_cycles` always, so each sound starts on the sample of its cycle. Advancing the FDC earlier than its next access changes nothing, since its events happen at their own times; the machine runs the same with the sounds on or off. Insert and eject are the host's calls, heard from the current cycle; disks put in before the machine first runs (cycle 0, the drives a host puts back at launch) were in their drives already and make no sound.
  - **Muted, off or restored.** While muted (warp) or off, what the drives do still changes the motor's state and the clamped disks, but plays nothing, and the spindle is at once at its speed; muting drops what is playing, so nothing plays on after a warp. Turning the sounds off fades them out over 20 ms. A restored snapshot sets the motor and the disks from the machine, with nothing playing.
  - **Levels.** At volume 1, the default, the recordings' levels are kept, halved (`GAIN`), against the speaker's square wave of ±0.5: the lever's latch peaks at about 0.5, the eject at 0.3–0.4, a step at 0.1 and a disk sliding at 0.01 RMS. A spinning disk is about 0.005 RMS with the hum, an empty drive less: quiet, since the motor never stops once started.
  - **Volume.** `Machine::set_drive_sounds_volume` scales every drive sound, from 0 to 2. A change is reached in about 20 ms, so a slider being dragged does not crackle; turning the sounds on starts them at the volume set.
  - **The idle motor faded out.** RomBios leaves the motor line on (MD §10.3), so the drives would be heard spinning for good. With `Machine::set_drive_idle_mute(true)` (off in a new machine) the motor's sounds, its hum, the disks' noise and their swish, fade out once the controller has had no command for 10 s (`IDLE_SECS`): evenly in dB, to −60 dB in 3 s, and then off. At the next command, step or motor-on they are back in 0.2 s. The spindle's speed, the motor line, READY and the disk's turns in the emulation are not touched: only what is heard. The time is counted in output samples, so it stands still while the audio is muted (warp) or the sounds are off; a restored snapshot starts it again. EKDOS at its prompt gives the controller no commands, so its drives fade out 10 s after the last disk access.
  - **Stereo.** The two drives sat side by side in the disk unit under the monitor; A: is taken to be the left one, as the order of the drive letters suggests (the unit's documents do not say which slot answers to which select line). In a photograph of the system their slots' centers are about 8 cm either side of the monitor's center line, some 8° off center for a user 60 cm away: a quarter of the way to stereo speakers at ±30°. A: is panned to −0.25 and B: to +0.25 (`PAN`), with constant power scaled so that the middle has unit gain in each channel: 1.18 and 0.79 (the ratio 1.5, 3.5 dB). Each drive is a source of its own (`Source`): its own noise and filter bank, frame resonances, knock tails and motor hum, so that the two drives heard at once stay apart. A step is the selected drive's (`DriveEvent::Step` carries it), an insert or eject the host's drive.
  - **Cost.** About 0.7% of one core while the disks spin and 1% during a seek (M1 Max, 48 kHz), nearly all of it the two drives' 42 filter stages each, since both motors turn together; nothing at all while the drives are silent.

### 4.6 Snapshots — `snapshot.rs`
A snapshot is the whole machine at one moment, in a file that can be shared between computers and platforms.

- **Container: a PNG.** The file is a valid PNG whose picture is a thumbnail of the screen, so any image viewer shows it. The state follows the image data, before `IEND`, in private ancillary chunks of type `juKu` (ancillary, private, safe to copy). Each chunk holds one section: a 4-byte tag, then the section's bytes.
  - Every PNG chunk has a CRC-32, so a damaged or truncated file is caught before anything is decoded. The CRC is computed bit by bit, with no table.
  - Numbers are little-endian, sizes are `u32`, and nothing depends on the host, so a file is the same on every platform.
  - Tags in capitals are the core's. Tags that start with a lowercase letter are the host's: `Machine::save_snapshot` stores them as given and `snapshot::section` returns them, and the core never looks inside.
  - **Thumbnail.** The host passes a PNG of its own (drawn as the user sees the screen), or the core makes one from the frame's active area: 1 bit per pixel, one pixel per Juku pixel, with stored (uncompressed) deflate blocks. That is about 10 KB.
- **Sections.** `SNAP` comes first and holds the format version (4). Then:

| Tag | Holds |
|---|---|
| `SYS ` | memory mode, `cycles`, `frame_cycle`, and the pulses delivered to PIT #1, PIT #3 ch1 and ch0 |
| `CPU ` | registers, flags, INTE, the EI delay, HALT, the INTA bytes |
| `RAM ` | 64 KB, compressed |
| `ROM ` | the 16 KB system ROM without the key click patch, compressed, then a flag for the patch |
| `EXPN` | the expansion window up to its last byte that is not FFh, compressed; nothing with no expansion ROM |
| `PIC `, `PIT1`–`PIT3`, `PPI1`, `PPI2`, `USRT` | every register and internal state of each chip |
| `FDC ` | registers, the command step and its times, the transfer buffer, motor, select, side, FM |
| `DRV0`, `DRV1` | head position, insertion time, and the disk if there is one: write protect and the image, compressed |
| `KBD ` | the matrix, SHIFT, CTRL and the DIP banks |
| `MOUS` | the backlog, the buttons, the poll number, the pending event |
| `RAST` | the raster (Video, §4.5): its time, line start and line, the address counter, HOR RTR, VER RTR and POF, the last active area, then the last frame and the one being drawn, 20 032 bytes each, compressed |

  - The ROM is stored as its file has it, and the key click patch is put back on restore. Once a snapshot is restored, the host tells which ROMs it holds by comparing bytes: `Machine::system_rom` gives the ROM as its file has it, and `Machine::expansion_rom` the expansion window without its trailing FFh (`juku_system_rom`, `juku_expansion_rom`).
  - A file is about 30 KB at the prompt with no disks: the ROM takes 13 KB, the core's thumbnail 10 KB, the raster's two frames 3 KB and the RAM 2 KB (up to 64 KB when it is full). A disk adds its image compressed: the two test images, nearly full, take 412 KB (`jukprog1.juk`) and 366 KB (`jukgame1.juk`), compressed in 4–5 ms; the empty sectors' runs take a few bytes each, and a disk full of compressed files would take its whole 800 KB. EKDOS booted from `jukprog1.juk` makes a 445 KB file.
- **Compression (`lz.rs`).** An LZ77 of the core's own, since the core takes no crates. The memory, the ROMs, the raster's two frames and the disk images are compressed (`Writer::packed`: the size, then the stream; `Reader::packed` checks the size against the most the section may hold before anything that large is made). The other sections are a few dozen bytes each.
  - **Stream.** A run of steps, each a literal count, that many literal bytes, and a match length; a length other than 0 is followed by its distance back. Every number is an unsigned LEB128 varint. A distance shorter than the length repeats the bytes it copies, so a run of one byte is a single step.
  - **Compressor.** Greedy: matches of at least 4 bytes, found through a hash of the next 4 bytes into a table of 32 768 last positions, at most 65 536 bytes back.
  - **Decoder.** It is given the size to make, the image's, and refuses as `Damaged` a stream that would write past it, reach back before its start, end short of it or leave bytes over, or that holds a varint over 32 bits.
  - **Older versions are refused** (`Old`). Version 3 compressed only the disks, version 2 nothing, and version 1 also kept the picture as `FRAM`, the active area alone, from before the raster. No release wrote them, so they are not read; version 4 files are refused by older cores as written by a newer format.
- **Not saved:**
  - The timer events (`pit1_event`, `sound_event`, `next_event`), which `schedule()` works out again, and the read and write maps, which the mode rebuilds.
  - The speaker's sample clock and filters. They depend on the host's sample rate, so the speaker restarts at the restored cycle (`Speaker::restart`) with its filters as they were, and its level is worked out again from PIT #3 OUT1 and PA4.
  - The drive sounds, which take the motor line and the disks from the restored machine, with nothing playing (§4.5).
  - The Caps Lock follower, which belongs to the host. A restore releases CAPS, which only the follower presses, and starts a new sync request.
  - What the host connected: the audio ring and its mute, the I/O log, the T3.2 clock. `frame_seq` keeps counting up, and the restored picture is published as a new frame, so the host sees it.
- **Restore is all or nothing.** `Machine::load_snapshot` parses and checks every section into new device structs, then swaps them in. On any error nothing changes:
  - `NotSnapshot`: not a PNG, or a PNG with no snapshot.
  - `Damaged`: a bad CRC, a truncated file, a missing or duplicate section, a section of the wrong length, or an impossible value.
  - `Version`: written by a newer format.
  - `Old`: written by a format before version 3.
  - **Checks.** Enum values and ranges (memory mode, PIT mode, access mode and phase, PIC init step, FDC step), the frame geometry, disk image sizes, and that no stored time is in the future of `cycles`, which is below 2⁵². They stop a damaged or crafted file from indexing out of bounds or underflowing a time, which would abort the app.
- **Disks** come back clean, whatever they were when the snapshot was taken, and keep their write protect. Which file a restored disk belongs to is the host's business (§6.8).
- Time is restored too, so `cycles` can go backwards. Disk rotation and the mouse polls are counted from power-on, so they carry on exactly.

---

## 5. FFI surface (`core/src/ffi.rs` ↔ `frontends/macos/src/Core.m`)

The C ABI is the macOS front-end's boundary. The Windows front-end is written in Rust and uses the crate's Rust API directly (§11.1).

The debug server has a few functions of its own, `juku_dap_*` in `debugger/dap/src/ffi.rs`, declared in the same `Core.m` (§12.2).

All functions are `#[unsafe(no_mangle)] pub extern "C"`. Handles are opaque pointers (`Box::into_raw`), and nothing panics across the boundary: with `panic=immediate-abort` the process aborts, which is acceptable.

```c
// ---- lifetime ----
JukuAudio   *juku_audio_new(uint32_t sample_rate);
void         juku_audio_free(JukuAudio *);
JukuMachine *juku_new(const uint8_t *rom, size_t rom_len, JukuAudio *audio);  // rom_len = 16384
void         juku_free(JukuMachine *);
void         juku_reset(JukuMachine *, bool cold);
bool         juku_load_rom(JukuMachine *, const uint8_t *rom, size_t rom_len);  // new ROM + cold reset (§4.3)
void         juku_system_rom(const JukuMachine *, uint8_t *out);                // 16 384 bytes, as the ROM's file has them (§4.6)
const uint8_t *juku_expansion_rom(const JukuMachine *, size_t *len);         // the window without its trailing FFh
int32_t      juku_expansion_check(const uint8_t *data, size_t len);             // 0, −1 size, −2 no program header
int32_t      juku_expansion_insert(JukuMachine *, const uint8_t *data, size_t len);  // as check; cold reset
void         juku_expansion_eject(JukuMachine *);                               // cold reset

// ---- run (main thread only) ----
// Runs until emulated time reaches the target; returns cycles executed.
uint64_t     juku_run_cycles(JukuMachine *, uint64_t cycles);
uint64_t     juku_cycles(const JukuMachine *);
const JukuFrame *juku_frame(const JukuMachine *);          // last completed frame

// ---- input ----
void juku_key(JukuMachine *, uint8_t juku_key_id, bool down);
void juku_keys_release_all(JukuMachine *);                 // on focus loss
void juku_caps_lock(JukuMachine *, bool on);               // guest Caps Lock follows `on` (§4.5)
void juku_mouse(JukuMachine *, int32_t dx, int32_t dy, uint8_t buttons);  // bit0 right, bit1 left
void juku_set_dip(JukuMachine *, uint8_t bank /*4|5*/, uint8_t value);
bool juku_set_key_click(JukuMachine *, bool on);           // ROM patch (§4.3); false if the ROM is unknown
void juku_set_t32_fast_clock(JukuMachine *, bool on);      // T3.2 at 1.2308 MHz, not once a frame (§4.5)
void juku_set_drive_sounds(JukuMachine *, bool on);        // the floppy drives' sounds (§4.5); off in a new machine
void juku_set_drive_sounds_volume(JukuMachine *, float volume);   // 0–2, 1 by default
void juku_set_drive_idle_mute(JukuMachine *, bool on);     // the motor fades out 10 s after the last command

// ---- disks ----
int32_t juku_disk_check(size_t len);                       // 0, or −1 for a size the drives do not take
int32_t juku_disk_insert(JukuMachine *, uint8_t drive, const uint8_t *data, size_t len, bool write_protect);
int32_t juku_disk_insert_blank(JukuMachine *, uint8_t drive);   // a formatted, empty disk (§4.5); 0 or −2
void    juku_disk_eject(JukuMachine *, uint8_t drive);
const uint8_t *juku_disk_data(const JukuMachine *, uint8_t drive, size_t *len);  // the disk in memory, for saving
bool    juku_disk_dirty(const JukuMachine *, uint8_t drive);     // written to since put in, saved or restored
void    juku_disk_clear_dirty(JukuMachine *, uint8_t drive);     // after a save
void    juku_disk_set_write_protect(JukuMachine *, uint8_t drive, bool on);
bool    juku_disk_write_protected(const JukuMachine *, uint8_t drive);   // a snapshot restores its own
uint8_t juku_drive_activity(const JukuMachine *);          // bit per drive: a command is running on it (title-bar indicator)

// ---- audio (audio thread) ----
size_t juku_audio_pull(JukuAudio *, float *left, float *right, size_t frames);   // fills with 0 on underrun
size_t juku_audio_fill(const JukuAudio *);                        // for rate control
void   juku_audio_mute(JukuMachine *, bool muted);                // warp: samples are computed but not queued

// ---- snapshots (§4.6) ----
typedef struct { uint8_t tag[4]; const uint8_t *data; size_t len; } JukuSection;   // host section
// The file to free with juku_snapshot_free; NULL for a bad thumbnail PNG or host tag.
// thumbnail NULL: the core draws the screen.
uint8_t *juku_snapshot_save(const JukuMachine *, const uint8_t *thumbnail, size_t thumbnail_len,
                            const JukuSection *sections, size_t count, size_t *len);
void     juku_snapshot_free(uint8_t *data, size_t len);
int32_t  juku_snapshot_load(JukuMachine *, const uint8_t *data, size_t len);      // nothing changes on error
int32_t  juku_snapshot_section(const uint8_t *data, size_t len, const uint8_t tag[4],
                               const uint8_t **out, size_t *out_len);            // *out NULL if absent
```

- **Error codes:** 0 = OK, −1 = bad size, −2 = bad drive. The snapshot calls return −1 = not a snapshot, −2 = damaged, −3 = made by a newer version, −4 = made by an older one.
- **Snapshot size cost.** The snapshot code is about 19 KB of the app's machine code (16.8 KB even at `opt-level = "s"`, so the release profile keeps its setting). The largest parts are the save and the load with every device's part inlined (3.4 and 3.2 KB), the FDC load (1.7 KB) and the core's thumbnail PNG (1.6 KB). The `Writer` and `Reader` methods are kept out of line, which saves 3 KB. The executable grows in 16 KB pages, from 195 KB to 217 KB.
- **Logging:** a `juku_set_log(void (*)(int level, const char *msg))` callback is routed to `os_log` in Objective-C. The core never prints.

---

## 6. macOS layer (Objective-C)

### 6.1 Threading and pacing
- **Single emulation thread: the main thread.** All `juku_*` calls except the audio pair happen there.
- **Driver:** the `NSView` display link (`[view displayLinkWithTarget:selector:]`). On each tick:
  1. `Δt = timestamp − last`, clamped to 100 ms so pauses and sleep don't cause a catch-up burst.
  2. `cycles = Δt × 2 000 000 × rate_trim`, where `rate_trim` is in [0.995, 1.005] and comes from audio-buffer feedback: target fill 2048 frames, small proportional adjust. This locks emulation speed to the audio clock and avoids clicks.
  3. `juku_run_cycles(m, cycles)`. At about 33 k cycles per 60 Hz tick this is far below 1 ms of CPU time.
  4. If `juku_frame(m)->seq` changed, upload it and render; otherwise re-render, because phosphor decay still advances. (As built: only while the picture can still change, see below.)
- **Window states:**
  - Occluded or minimized: emulation keeps running on a 50 Hz `NSTimer` fallback, so audio and disk I/O continue, and rendering is skipped.
  - Background (not key): continues, and keys are released.
- **As built: when a tick draws.** Drawing and presenting on every tick was the main cost of the app, far above the emulation itself, so `displayLinkFired:` draws only when all of these hold:
  - **The window is in view** (`NSWindowOcclusionStateVisible`). A fully covered window still gets display link ticks, so this has to be checked; a minimized one gets none.
  - **The picture is settling.** Each new Juku frame is compared with the last one looked at (its active area and the raster). A difference starts a settling period of 0.3 s, the time the afterglow takes to fade below one output step. A resize, a settings change, pausing or resuming, and the window coming back into view start one too (`setNeedsRedraw`).
  - **There is something to draw.** A Juku frame that has not been drawn yet is drawn, but never within 12 ms of the last draw, which limits warp speed to 60 draws a second. With no new frames (paused, raster stopped) the fade is drawn every 30 ms.
  - Outside a settling period nothing is drawn, and `nextDrawable` is not called.
  - **Phosphor time.** The renderer is given the time the phosphor has had to decay. It counts from the last draw, or from the last tick on which the picture was settled and in view. After a hidden spell it is long, so nothing of the old picture is left; after a settled spell it is short, so a pixel that goes dark still leaves its afterglow.
  - **The cursor blinks.** RomBios and EKDOS change the picture every 13 frames (0.26 s), which is inside the settling period. A prompt is therefore drawn continuously, at the Juku's 50 frames a second. Only a picture without a blinking cursor, or a paused machine, stops the drawing.
  - **Measured** (M1 Max, 120 Hz display, 960 × 720 pt window, monitor prompt; the "before" column draws every tick):

| | Before | After |
|---|---|---|
| Draws per second | 120 | 50 |
| Juku process CPU | 16% | 11% |
| WindowServer CPU | 47% | 23% |
| GPU power, window | 0.75 W | 0.33 W |
| GPU power, full screen | 2.92 W | 0.54 W |
| CPU + GPU power, window | 0.94 W | 0.54 W |
| CPU + GPU power, full screen | 3.28 W | 0.66 W |
| Paused | as above | no draws, 3% CPU, 0.1 W (the idle level) |
| Covered by another window | as above | no draws, 6% CPU |

  - At 120 draws a second in full screen the GPU ran at its top clock (1296 MHz), which is where the 2.92 W came from; at 50 it stays at the lowest (389 MHz). CPU power was 0.1–0.4 W in every case: these CPU percentages are of efficiency cores at a low clock.
  - Power is from `powermetrics` (medians of one-second samples); the machine idles at 0.09 W with no app running.

### 6.2 Window and menus
- **Window:**
  - One `NSWindow` whose content view is `EmuView` (with a `CAMetalLayer`).
  - The content aspect is locked to **4:3** (`contentAspectRatio`). It opens with the size and position it had when last used: AppKit keeps the frame under `NSWindow Frame Juku` (`frameAutosaveName`) and moves it onto a screen if that one has gone. The first time, it opens centered at the largest of the View menu's sizes that fits on the screen, 3× if it can; the minimum is 1× without the bezel margin, 383 × 287 pt.
  - Fullscreen is allowed (Ctrl-Cmd-F).
  - In full screen the cursor hides on entering, shows when the mouse moves, and hides again after 3 s without movement (`EmuView`, `setHiddenUntilMouseMoves`). A tracking area brings the mouse-moved events. The countdown is one timer, in the default run loop mode so that it waits while a menu is open, and is not restarted on every event: when it fires early it waits on for the rest. The cursor stays shown while the menu bar is revealed, while a button is held, when it is on another screen, and when the app is not active; a captured mouse hides it anyway (§6.5). Checked in the app with HID-level moves and screenshots with and without the cursor: `CGCursorIsVisible` now always says hidden.
  - The title shows "Juku", plus drive activity dots, e.g. `Juku — A●`.
- **Menus** (localized en/et via `Localizable.strings`, built from `resources/strings`):
  - **Juku:** About, Settings…, Quit.
  - **Machine:**
    - Reset (Cmd-R) — warm, like the hardware button
    - Power Cycle (Shift-Cmd-R) — cold
    - System ROM submenu: one item per bundled ROM, the user's last ROM file, and Other…; the ROM in use is checked (§6.7)
    - Expansion ROM submenu: None, the user's last expansion ROM file, and Other…; the one plugged in is checked (§6.7)
    - Open Snapshot… (Cmd-O), Save Snapshot… (Shift-Cmd-S), Quick Save (Cmd-S), Quick Load (Cmd-L) (§6.8)
    - Pause (Cmd-P)
    - Warp Speed (Cmd-W): runs up to 20× and mutes audio
  - **Disks:** one submenu per drive, titled with the drive letter and the disk's name, `A: prog.juk`, or the letter alone for an empty drive, `B:`. The name is that of the disk's file (§6.3), or for a disk from a snapshot without its file, the name the snapshot gives (§6.8). The name of a disk that is not saved to a file (§6.3) is in square brackets, and a dot follows the name of one the running program has written to since it was read or saved: `B: [Untitled.juk] •`. The words stay in the submenu, so the menu keeps its width. The titles are set as the menu opens (`DiskDrives` is its delegate), with the file's path as the tooltip. Each submenu has:
    - A status line, grayed, with a separator under it: “Not saved to a file” for a disk that is in no file yet (§6.3), “Edited” for one with the dot, or both, “Not saved to a file · Edited”. It is hidden when neither is so, and for an empty drive (`DiskDrives`, `statusItemsForDrive:`)
    - Insert… (Cmd-1 in A:, Cmd-2 in B:), using `NSOpenPanel` filtered to `.juk`, `.img`, `.dsk` and `.cpm`
    - New Disk, a formatted, empty disk without a file (§6.3)
    - Recent Images, which inserts in that drive
    - Save Disk…, which saves the disk in memory to a file the user picks (§6.3); no shortcut, disabled for an empty drive
    - Eject (Shift-Cmd-1 in A:, Shift-Cmd-2 in B:)
    - Write Protect, a toggle, off for every disk put in. It keeps the running program from writing to the disk; the file is safe either way (§6.3)
    - Before leaving the drive, a changed disk asks to be saved (§6.3)
  - **View:**
    - CRT Effects on/off (Cmd-E)
    - Size: 1×, 1.5×, 2×, 2.5×, 3× (§7.3a)
    - Enter Full Screen
  - **Input:** Capture Mouse (Cmd-M), Keyboard Mapping help.
- **Drag and drop** of an image onto the window inserts it in A:. Holding Option inserts it in B:. A dropped snapshot is opened (§6.8).
- **Info.plist:** `CFBundleDocumentTypes` declares `.juk` (role Viewer), so Finder's "Open With" inserts the image into A:.
- **As built:**
  - **Pause** stops the machine and keeps the picture. Drawing stops once the phosphor has settled (0.3 s, §6.1) and resumes for a resize or a settings change, so a paused machine costs about 3% CPU.
  - **Warp** runs slices of 20 000 cycles until 20× the elapsed time is emulated or 75% of the tick (at most 15 ms) is used. It reaches the full 20× at about a third of one core. The subtitle shows the measured speed, e.g. "Warp Speed (20×)". Audio is muted in the core (`juku_audio_mute`): the ring drains, and after warp it re-primes at the normal latency instead of playing a backlog.
  - **Cmd-W** is Warp Speed only in the emulator window; in Settings or Keyboard Mapping it closes that window.
  - **Title and subtitle:** the title is `Juku`, plus `A●`/`B●` while a disk command runs, held for 0.3 s so short commands are visible and a run of sectors does not flicker. The subtitle shows the mouse-release hint, then a note such as "Snapshot Saved" (§6.8), then Paused, then Warp Speed.
  - **Document types:** `.juk` is an exported type `io.github.sinijarv-juku.juku-emu.disk-image` (role Editor, rank Owner, since Juku saves disks to them). `.cpm` and `.dsk` are an imported type `io.github.sinijarv-juku.juku-emu.raw-disk-image`, and `.img` (`com.apple.disk-image-udif`) is listed too, both with rank Alternate, so they offer Juku under Open With without taking over as the default. `.jukusnap` is the exported snapshot type (§6.8).
  - **Recent Images** (in each drive's submenu) holds up to 10 bookmarks in the `RecentDisks` default, newest first; both drives show the same list. Missing files are left out. Images are also reported to `NSDocumentController` for the Dock menu.
  - **Extra menus:** Hide, Hide Others and Show All in the app menu, and a Window menu. Minimize has no shortcut because Cmd-M is Capture Mouse.
  - **Keyboard Mapping** (Input menu) is a window listing the host keys whose Juku legend differs.
  - **Settings** gained a Mouse Speed slider (0.1–2 counts per point).
  - **Localization:** everything the app draws is in en and et. Items that macOS adds to the menus itself (Window → Move & Resize, Quit and Keep Windows) stay English, because macOS has no Estonian localization. Settings → Language picks the language (§6.7).

### 6.3 Disks in memory (`Disks.m`)
The disk in a drive lives in memory, as in Hatari: the running program writes only to it. Its image file is read when the disk goes in and written only when the user saves, always through a save panel, so the user sees where it goes and can give it a new name. Nothing is written behind the user's back, and opening a snapshot never has to work out which file a disk belongs to (§6.8).

- **A drive's disk.** The core keeps the disk and its changed flag (§4.5). `DiskDrives` keeps, per drive:
  - **Its file:** where the disk was read from or last saved to, or none for a disk from a snapshot whose file is not on this Mac. The file gives the disk its name in the Disks menu, is where Save Disk… and the save alert's panel open, and is what the `Drives` default puts back at the next launch. It is not kept in step with the disk.
  - **Its name:** the file's (`displayNameAtPath:`), or for a disk without a file the snapshot's name for it (§6.8).
  - **Write Protect.**
  - **Not saved to a file** (the status line, §6.2): set for a new disk and for every disk a snapshot brings, whether or not its file is still there, and cleared when the disk is read from a file or saved. Such a disk is in no file as it is; the snapshot still holds one from a snapshot, so ejecting it unchanged does not ask.
- **Changed** is the core's flag (`juku_disk_dirty`): the running program has written to the disk since it was put in, saved or restored from a snapshot. A write of the bytes a sector already held counts too. Write Protect is not part of it.
- **Insert:** Insert…, Recent Images, a drop or Finder's Open With read the whole file into `NSData`, check its size (`juku_disk_check`) and call `juku_disk_insert` with Write Protect off. A disk already in the drive leaves first, once the new one has passed the check, as on Eject, and stays if the user cancels. A file that cannot be read or has the wrong size is reported in an `NSAlert`, and the drive keeps its disk. The same image may be in both drives: each has a disk of its own, and only a save writes to the file.
- **New Disk** puts in a formatted, empty disk (`juku_disk_insert_blank`, §4.5), once the disk in the drive may go. It has no file and is named “Untitled.juk” (“Nimetu.juk”), so the first save proposes that name in Documents. It is unchanged until the running program writes to it.
- **Save Disk…** opens an `NSSavePanel` on the disk's file, its folder and name, or for a disk without a file on its name in Documents, filtered to the disk image types. The panel asks before replacing a file. When it closes the disk is written atomically (`NSDataWritingAtomic`) and is no longer changed (`juku_disk_clear_dirty`); the saved file becomes the drive's file and goes to Recent Images, and the `Drives` default is saved. A failed write is reported ("Changes to “%@” could not be saved.") and the disk stays as it was.
- **Leaving the drive.** Before a changed disk leaves its drive an alert asks: on Eject, on an insert into the drive from anywhere, when a snapshot is opened (§6.8) and on quit. An unchanged disk leaves without asking.
  - "Do you want to save the changes to “prog.juk” in A:?", with "Your changes are lost if you don't save them." under it, and **Save…**, **Don't Save** (Cmd-D) and **Cancel** (Esc).
  - **Save…** is Save Disk…; when the panel is canceled or the write fails, the alert asks again. **Don't Save** lets the disk go. **Cancel** keeps it in the drive and stops what was being done: the insert, the eject, the opening or the quit.
  - Two changed disks are asked about in turn, A: first. A Cancel at B: leaves A: as the user answered it: saved, or still in its drive.
  - The alerts and the save panel are app-modal (`runModal`), as the disk alerts already are, so nothing else changes the drives while they are up.
- **Write Protect** keeps the running program from writing to the disk; EKDOS sees the disk's write-protect tab. The file is never at risk, so it is off for every disk put in, and the `Drives` default keeps the user's choice across launches.
- **Quit:** `applicationShouldTerminate:` asks about each changed disk before it replies, and Cancel answers `NSTerminateCancel`, which stops a log out or shut down too. Closing the window quits (`windowShouldClose:` sends `terminate:`), so the disks are asked about while the window is still there.
- **A crash loses the changes** since the last save, as does a forced quit. A recovery copy in Application Support was considered and left for later.
- **Across launches:** the `Drives` default holds one entry per drive with a file: a bookmark of the file and Write Protect (`Bookmark`, `WriteProtected`). An empty entry stands for an empty drive or a disk without a file, so such a drive is empty at the next launch. It is saved on every insert, eject, save and Write Protect change. At launch `restoreDrives` reads the files back before the machine runs, so it boots from A: as before; nothing is lost by that, since quitting saved or let go of every change. An image that cannot be found or no longer loads is reported once the app is up, with the path kept in the bookmark, and its drive stays empty. Restoring does not reorder Recent Images. An image opened from Finder at launch still goes into A:, replacing the restored one.
- **Checked in the app**, on copies of `jukprog1.juk`: after `SAVE 2 ZZ.COM` at EKDOS, Shift-Cmd-1 asked about `prog.juk` in A:; Cancel kept the disk, and Save… opened the panel on `prog.juk` in its folder, where a new name saved `prog2.juk` holding `ZZ.COM`, left `prog.juk` unchanged and emptied A:. With `prog2.juk` in A:, Quick Save, `ERA ZZ.COM` and Quick Load asked about the disk; Don't Save loaded the snapshot, and `DIR ZZ.COM` found the file again. Writing again and Cmd-Q asked; Cancel kept Juku running and Don't Save quit it, the file unchanged. New Disk in B:, `SAVE 1 B:NEW.COM` and Shift-Cmd-2 asked about “Untitled.juk” in B:, and Save… wrote an image holding `NEW.COM`. At the next launch A: had `prog2.juk`; after it was renamed in Finder, a Quick Load left A: without a file, named `prog2.juk`, and Save Disk… proposed that name in Documents. The close button quit Juku with nothing changed, and the `Drives` default then held no file for A:.

### 6.4 Keyboard (`Keymap.m`, `EmuView.m`)
- `EmuView` is first responder. `keyDown:` / `keyUp:` / `flagsChanged:` are translated with a **positional** table from macOS virtual keycodes to Juku key IDs. Events with Cmd held go to the menus and are never forwarded. Key repeat events (`isARepeat`) are ignored, because the BIOS does its own auto-repeat.
- **Default table:**
  - Letters, digits, `,` `.` `/` `;` and Space map to the same positions as on the Juku keyboard (MD §6.3).

  | Host key | Juku key |
  |---|---|
  | `[` | Ü |
  | `]` | Õ |
  | `;` | Ö |
  | `'` | Ä |
  | `\` | `* :` |
  | `-` | `− =` |
  | `=` | `Ю @` |
  | `` ` `` and ISO § | LAT/RUS |
  | Return / keypad Enter | RETURN |
  | Backspace | ← |
  | Forward Delete | DEL |
  | Home | ERASE |
  | End | `{ [` |
  | Page Down | `} ]` |
  | Page Up | `\ ^` |
  | arrows | arrows |
  | F1–F8 | F1–F8 |
  | Esc, Tab | ESC, TAB |
  | Shift (L/R) | SHIFT |
  | Control (L/R) | CTRL |
  | keypad digits and operators | the corresponding keys |

  - **`; +`** is an extra key to the right of `/` on the Juku, with no host key in that position. It is mapped to **Right Option** and to keypad `+`.
  - The final table is in `Keymap.m`, with a comment per row.
- **Caps Lock:** the guest Caps Lock follows the host's at all times. Each tick passes `[NSEvent modifierFlags]`'s Caps Lock bit to `juku_caps_lock`, which works whether the window has focus or not. The core presses CAPS while the guest state differs (§4.5), so the two match at startup, after a reset and after any host change. `flagsChanged:` ignores Caps Lock.
- **Focus:** on focus loss, call `juku_keys_release_all`.

### 6.5 Mouse
- **Capture.** Cmd-M (Input → Capture Mouse), or a click in the window **while the running program has IR6 unmasked**, i.e. actually uses the mouse. This replaces the "click to capture" setting, so clicking the window at the monitor or in EKDOS never hides the cursor. While captured:
  - hide the cursor
  - `CGAssociateMouseAndMouseCursorPosition(false)`
  - read raw `deltaX`/`deltaY` from `mouseMoved:` and the dragged events
  - map buttons: left → left, right → right
- **Release:** Cmd-M, Cmd-Tab or focus loss. While captured the title reads "Juku – Press ⌘M to release the mouse", and the hidden cursor is parked in the middle of the view.
- **Verified in the app:** a click captured the mouse in gtr.com, HID-level movement moved its cursor, and drags drew lines. Mouse events posted to a process with `postToPid` never reach AppKit views; test tools must post at the HID tap.
- **Scaling:** host points to Juku counts with a sensitivity setting, default 1 count per 2 points. The remainder accumulates as a fraction, and the result goes to `juku_mouse`.

### 6.6 Audio (`Audio.m`)
- An `AudioComponent` of type `kAudioUnitSubType_DefaultOutput`, 48 kHz, stereo Float32, non-interleaved, with a render callback that calls `juku_audio_pull` with the two channels' buffers.
- No locks and no Objective-C messaging in the callback.
- Pause and Warp mute by having the callback output silence. (As built: the ring simply runs dry; warp also stops the core queueing samples.)
- **As built:**
  - The unit is fed 2-channel non-interleaved Float32. Before the drive sounds were panned, the ring was mono and its samples were copied to both channels, because a mono stream on the default output plays on the left channel only.
  - The ring re-primes after an underrun: output stays silent until 2048 samples (about 43 ms at 48 kHz) are queued, which sets a stable latency.
  - If no output device can be opened, the machine runs on the host clock alone, without the rate trim.
  - A 20 ms `NSTimer` runs the machine whenever the display link has not ticked for 40 ms (window hidden or minimized).
  - Emulation costs about 3% of one core in the app (about 9% before the PIT pulses were batched, §4.1). The same work run flat out takes 0.6% of a performance core: a thread that is this lightly loaded runs on slow cores, which inflates the figure.

### 6.7 Settings (`Settings.m`)
- `NSUserDefaults` keys: CRT on/off and its parameters (§7.3), the window frame (§6.2), mouse sensitivity, click-to-capture, key click, drive sounds, recent disks, the disks in the drives (§6.3), the snapshot key (§6.8), DIP banks (advanced), and audio volume.
- **Key click:** the Key Click checkbox in the General pane's Sound row (`KeyClick`, on by default, as RomBios turns it on at startup). The app delegate passes it to `juku_set_key_click` at startup, before the machine runs, and again on every `NSUserDefaultsDidChangeNotification`. The call is cheap and idempotent, and the change applies at the next key press. Its result says whether the ROM in the machine has the click code the patch needs (§4.3). The app delegate keeps it and hands it to the Settings window, at creation and after every call. When it is false the checkbox is grayed out and a note under it says that this ROM's key click cannot be changed. The note's grid row is hidden otherwise. When the row appears or goes while the General pane is shown, the window is refitted to it, keeping its title bar in place. The refit waits for the next turn of the run loop: the call can come from a defaults change that AppKit posts while it measures a pane's fitting size (a text class registers defaults when first used), and measuring again inside that aborts in SwiftUI's text layout.
- **Drive sounds:** under Key Click in the Sound row, the Drive Sounds checkbox (`DriveSounds`, on by default), then a Drive Volume slider from 0 to 2 (`DriveSoundsVolume`, 1 by default, in the middle) and a Fade Out the Motor When Idle for 10 s checkbox (`DriveIdleMute`, off by default). The slider and the second checkbox are grayed out while Drive Sounds is off, and so is the slider's label (`disabledControlTextColor`). The app delegate passes the three to `juku_set_drive_sounds`, `juku_set_drive_sounds_volume` and `juku_set_drive_idle_mute` with the key click, at startup and on every defaults change; they take effect at once (§4.5).
- **System ROM:** `SystemROM` names the ROM: a bundled one by its file name without `.bin` (`ekta37` by default), or the user's file by its absolute path. `Roms.m` lists the bundled ROMs with the version and serial number each prints at power-on, so the menu and the Settings pop-up read "RomBios 3.43m #0037 (Default)".
  - **Switching.** The app delegate compares the setting with the ROM in the machine on every defaults change. When they differ it reads the ROM and calls `juku_load_rom`, which cold-starts the guest, then applies the key click again. A name not in the table, or a file that cannot be read or is not 16 KB, puts the setting back to the ROM in use; for a file an alert says so. At launch such a value is removed, so the default applies, and a missing file is reported once the window is up. A note under the pop-up says that changing the ROM restarts the machine.
  - **ROM files.** Other… (in the menu and at the end of the pop-up) opens an `NSOpenPanel` for `.bin` and `.rom` files, as a sheet on the key window. A file that is not 16 KB is refused with an alert and never reaches the setting. A good one is stored in `SystemROMFile` and in `SystemROM`. Choosing the same file again reloads it, since it may have changed. `SystemROMFile` keeps the last file as an item of its own, named by its file name with the path as a tooltip, so the user can go back to it after picking a bundled ROM. The path is stored as is; the app is not sandboxed. The key click works with a file only if it has the click code laid out as in the bundled ROMs (§4.3); otherwise Key Click is grayed out, as above.
- **Expansion ROM:** `ExpansionROM` holds the path of the plugged-in expansion ROM, absent for none (the default). No expansion ROM is bundled; the user picks a file.
  - **Choosing.** Machine → Expansion ROM and the Settings pop-up below System ROM work as for the system ROM: None, the last file (`ExpansionROMFile`) and Other…. The panel's file is read and checked with `juku_expansion_check` before it reaches the setting. A file over 32 KB, or one without a program header, is refused with an alert that says which. The note under the pop-up says to start it with `A` at the `*` prompt and that changing it restarts the machine.
  - **Applying.** After the system ROM, `applySettings:` compares the setting with the plugged-in path and calls `juku_expansion_insert` or `juku_expansion_eject`, each a cold start. A file that has gone, or no longer passes the check, is reported and the setting goes back to what is plugged in; at launch that is none. The setting is kept across launches, so the app starts with the same expansion ROM plugged in.
  - **With JMON 3.3** `A` loads the window too, but JBASIC then stops with `?14` errors; JMON has the same BASIC built in, started with `B`. MAME pairs the cartridge with the RomBios BIOSes only.
- **Timer T3.2:** below Expansion ROM, a Clock at 1.23 MHz checkbox (`T32FastClock`, off by default), with a note that RDEM needs it and that the RomBios stopwatch then runs too fast. The app delegate passes it to `juku_set_t32_fast_clock` with the key click, at startup and on every defaults change; it takes effect at once, without a restart (§4.5).
- **The Settings window** has three panes, chosen from a toolbar in the preference style (`NSWindowToolbarStylePreference`):
  - **General** (`gearshape`): Mouse Speed, Sound (Key Click, Drive Sounds, Drive Volume, Fade Out the Motor When Idle for 10 s) and Language.
  - **Display** (`display`): CRT Effects, the CRT sliders, 3D Frame, Brightness and Restore CRT Defaults (§7.3a).
  - **Machine** (`cpu`): System ROM, Expansion ROM and Timer T3.2.
  - Each pane is an `NSGridView` form with 20 pt margins, written in code with no nibs, like the rest of the app. The window's title is the pane's name. Choosing a pane fits the window to it, animated and keeping the title bar in place; an empty view stands in while the window resizes, so the new pane's constraints never fight the animation. The window opens on the pane it last showed in this run, General at first.
- **Language:** a pop-up with System Default, English and Eesti (the languages named in their own language). It writes `AppleLanguages` in the app's own domain (`@[@"et"]`), or removes it for System Default. This is the key that System Settings → Language & Region → Applications writes, so the two always agree, and `NSBundle` reads it at launch to pick the `.lproj`. The choice therefore takes effect on the next launch, which the window says under the pop-up. The current value is read with `persistentDomainForName:`, because a plain lookup falls through to the system-wide list. A value with a region (`et-EE`) matches its language.

### 6.8 Snapshots (`Snapshots.m`, `Main.m`)
- **Menus** (Machine): Open Snapshot… (Cmd-O), Save Snapshot… (Shift-Cmd-S), Quick Save (Cmd-S) and Quick Load (Cmd-L). Quick Save and Quick Load use one file, `~/Library/Application Support/Juku/Quick Save.jukusnap`, and Quick Load is disabled until it exists. Both show "Snapshot Saved" or "Snapshot Loaded" in the subtitle for 2 s.
- **Saving.** The snapshot is taken when the item is chosen, before any panel opens. It holds the disks as they are in memory, changes and all, so nothing is written to their files. The save panel offers `Juku <date time>.jukusnap`.
- **`meta` section.** UTF-8 lines of `key=value`, readable on any platform: `app` (name and version), `saved` (UTC, ISO 8601), `rom` (a bundled ROM's name or the ROM file's name), `expansion`, `disk0` and `disk1` (the disks' names, §6.3). It holds no paths: they are of no use on another computer, and they tell more about the user than a shared file should.
- **`mac ` section: the files on this Mac, sealed.** The files the snapshot came from: `Disks`, one entry per drive with the path of the disk's file (`Path`, absent for none), and the paths of ROM files in `SystemROM` and `ExpansionROM`. They are a binary plist, encrypted with AES-256-CBC (random IV, PKCS#7 padding) and then authenticated with HMAC-SHA256 over the version, IV and ciphertext, both from CommonCrypto (`CCCrypt`, `CCHmac`), which is part of macOS, not of the app. The section is `version (1), IV, ciphertext, MAC`.
  - **Key.** 64 random bytes from `arc4random_buf`, made on first use and kept in the `SnapshotKey` default: the first half encrypts, the second authenticates. Not in the Keychain: an ad-hoc signed build changes its signature on every build, and the Keychain would ask for access each time.
  - **No identifier in the file.** The planned installation ID turned out to be unnecessary: the app knows its own sections because their MAC checks out with its key. Another Mac, or this one after its preferences were deleted, cannot tell whose a section is, so shared snapshots cannot be linked to one another or to the computer they came from. A section that does not check out is ignored, and the snapshot is restored as one from another computer.
  - **Cost.** About 3 KB of machine code; CommonCrypto lives in `libSystem`, which every app links anyway.
- **Opening.** From the menu, Finder (Open With, double-click), the Dock's recent documents, or dropped on the window. The file is read and its sealed section opened. Then:
  - **The disks in the drives.** For each drive whose disk is changed, the save alert asks (§6.3); Cancel stops the opening. Quick Load is opened the same way.
  - **The machine.** `juku_snapshot_load` runs; an error is reported by name: not a snapshot, damaged, made by a newer or an older version, or unreadable. The machine and its disks are then as they were, a disk the user chose not to save included, since nothing has replaced it.
  - **ROMs.** The ROM in the snapshot, read back from the restored machine (`juku_system_rom`), is matched by its bytes to the bundled ROMs, then to the file it came from (from `mac `), then to the ROM files in `SystemROM` and `SystemROMFile`, then to the files Juku keeps in `Application Support/Juku/ROMs`. A ROM found nowhere is written there, named after its `meta` name. The expansion window is matched the same way, ignoring trailing FFh; a kept copy is padded with FFh to the length in its program header, so it passes the check. The settings are pointed at what was found, and the app delegate's record of the ROMs in the machine is set first, so the defaults change does not load a ROM or cold-start the machine. The settings then describe the machine again, and its menus and the Settings window show the right ROMs.
  - **Key click.** The setting applies again, whatever the snapshot had.
  - **Disks.** Each drive has the snapshot's disk, unchanged, with its Write Protect, from the core. The file comes from the sealed section: the file is the path there if a file is at it, whatever it holds now. Without the section (a snapshot from another computer), or with nothing at the path, the disk has no file and takes its name from `meta` (`Disk A.juk` without one), and Save Disk… proposes that name in Documents. Nothing is compared and nothing is asked: the disk lives in memory, and the user saves it where they like (§6.3). Changes the disk had in the snapshot, never saved to its file, live on in the snapshot file: ejecting does not ask about them, and the next launch reads the drive's file as it is. The `Drives` default is saved.
  - **The view.** Keys are released, the warp speed measurement and the cycle remainder start again (time may have gone back), and the picture is redrawn.
- **Thumbnail.** The snapshot's picture is the screen as the window shows it: `-[Renderer pngForFrame:width:height:]` runs the window's decode, phosphor and CRT passes (or the flat pass with CRT Effects off) into textures of its own, so the window's phosphor is left alone, with no afterglow, at 640 × 480. `NSBitmapImageRep` encodes it as PNG, about 50 KB, and it goes to `juku_snapshot_save` as the thumbnail. If it cannot be drawn, the core's 1-bit picture is used.
- **Document type.** `.jukusnap` is an exported type, `io.github.sinijarv-juku.juku-emu.snapshot`, conforming to `public.data` and `public.content`, with role Editor and rank Owner.
  - **Not `public.png`**, which was the plan: Finder then hands snapshots to the system's image thumbnailer, which passes the snapshot type to ImageIO as a hint and fails ("is not specifying image format"). `qlmanage -t` makes a thumbnail all the same, so it is no test. Without the conformance Preview is not offered under Open With; a snapshot renamed to `.png` still opens in any image viewer.
- **Finder thumbnails** come from a Quick Look thumbnail extension, `JukuThumbnail.appex` (`frontends/macos/src/thumbnail/`, §3.2). It names the snapshot type in `QLSupportedContentTypes`, reads the file with ImageIO told it is a PNG, which skips the private chunks, and draws the picture scaled to the size asked for. Quick Look's drawing context is in pixels, the requested size times its scale, and is not scaled to points, so the picture is drawn over the context's whole clip box. The extension is 52 KB, of which 576 bytes are code; the rest is the four 16 KB segments of an arm64 binary.
  - **Development builds.** The thumbnail agent (`com.apple.quicklook.ThumbnailsAgent`) reads the list of extensions when it starts and does not notice one added later; it ignores SIGTERM, and System Integrity Protection refuses `launchctl kickstart`. After the extension is first installed, thumbnails appear only once the user has logged out and in again. Of two copies of the app, the system uses the extension in `/Applications`.

- **Verified in the app.**
  - A quick save after a monitor command, then a power cycle and a quick load, brings back the screen, without a cold start. Through the panel, the snapshot is saved with its extension.
  - A snapshot made on RomBios #0031, loaded while #0037 was selected, puts the setting back to #0031 and shows its prompt 0.3 s later. A snapshot made with a ROM file that was then deleted keeps a byte-identical copy in `Application Support/Juku/ROMs` and points the setting at it.
  - The snapshot file holds no path and no user name, only the file names in `meta`. With another key in `SnapshotKey`, standing in for another Mac, the same snapshot ignores `mac `.
  - Thumbnails, with the app in `/Applications` and after logging in again: `QLThumbnailGenerator` returns a 256 × 192 thumbnail of the snapshot's picture, and Finder's icon view shows the screens of snapshots, including files it had shown with a generic icon before.
- **Size.** With the snapshot code linked, the app's machine code grew from 70.7 KB to 102.8 KB: 19 KB for the core's snapshot code (§5), 10 KB for saving and opening and 3 KB for the sealed section; the import sheet then took it to 113.7 KB, and the thumbnail drawing to 115.4 KB. Unstripped, the executable went from 195 KB to 278 KB, growing in 16 KB pages. Release builds are stripped (§3.2), which brings it to 229 KB, plus the 52 KB extension.

---

## 7. Rendering and CRT emulation (Metal)

### 7.1 Inputs
- **`videoBytes`:** an `R8Uint` texture of the core's raster, 64 × 313 bytes, re-uploaded only when `seq` changes. It stays 1 bit per pixel; the shader expands it.
- **Raster canvas:** the picture is shown in a virtual **52 µs × 287-line** visible raster (PAL-like), which starts 10.5 µs after the start of horizontal sync and 23 lines after the start of vertical sync. The core's raster starts at the start of both sync pulses (§4.5), so it goes in at (−84, −23) texels, the `active` uniform, and the borders and aspect come out as the timers make them:
  - The RomBios picture = 40 µs × 241 lines, from 16 µs and 47 lines after sync, giving a pixel aspect ratio of about 0.92.
  - In 384×200 mode it becomes 48 µs × 200 lines.
  - The canvas resolution is 8 px/µs horizontally, so 416 × 287 texels.

### 7.2 Passes
1. **Decode** (fragment shader, 416 × 287 `R16Float` target): look up the bit with MSB = left; outside the core's raster the value is 0. POF is in the raster already, as blank bytes. Output is linear beam intensity, 0 or 1.
2. **Phosphor persistence** (ping-pong 416 × 287 `RG16Float` targets):
   - `p = max(signal, prev × exp(−Δt / τ))`, with τ ≈ 5 ms for the white P4 phosphor.
   - G holds a slower afterglow component for a faint ghost trail.
   - This also hides the 49.92 Hz vs 60/120 Hz cadence mismatch.
3. **CRT composite** (fragment shader, drawable-sized):
   - **Geometry:** barrel curvature (k ≈ 0.04), overscan crop, rounded-corner glass mask, and the monitor frame outside the glass: a dark charcoal front, set by the Frame setting, standing in front of the glass, with its inner wall seen in perspective and lit by the face. With the 3D Frame setting off, the frame is flat, with a recess where it meets the glass.
   - **Scanlines:** a Gaussian vertical beam profile per raster line, with the sigma widened by brightness (beam bloom). With only 287 lines the dark gaps between lines are visible in dark and dithered areas, as photos and videos of the real 50 Hz monitor show; in solid white the bloom closes them (MD §7.4).
   - **Horizontal:** each pixel is a box softened by the beam spot, and widened by bloom in proportion to its brightness. White on black comes out bolder than black on white, as photos and videos of the real monitor show (MD §7.4).
   - **Glow:** a small 9-tap blur of the phosphor texture, added at the Glow setting (default 0.085).
   - **Background:** the unlit face is not black (MD §7.4). A gray for room light on the glass is added across the whole glass, darkened toward the bezel.
   - **Tint:** the white P4 phosphor color `#E8F0FF`, then a vignette and a gamma/sRGB encode. (The planned slight noise was dropped, §7.3a.)
4. **"CRT off" path:** nearest-neighbor or sharp-bilinear scaling of the decode output, with the correct aspect and no effects.

### 7.3 Uniforms
`CRTUniforms` (defined in `ShaderTypes.h`):
- `canvasSize`, `activeRect`, `outputSize`, `dt`, `tau1`, `tau2`
- `curvature`, `scanlineSigma`, `bloomAmount`, `glowAmount`, `vignette`
- `phosphorRGB`, `brightness`, `contrast`
- `background` (the unlit face color), `frame` (the monitor frame color), `bezel3d` (the 3D Frame setting)

The Settings window exposes a curated subset.

### 7.3a As built
- **Uniforms:** `CRTUniforms` is 144 bytes, defined once in `ShaderTypes.h`. Persistence decays are computed on the CPU per draw as `exp(−Δt/τ)`, where Δt is the phosphor time `EmuView` passes in (§6.1).
- **Textures and passes:** the beam canvas is `R16Float` and the two persistence textures are `RG16Float`, all 416 × 287. The persistence textures are cleared once at startup. They have mipmaps, which `fsh_downsample` redraws after the persistence pass when CRT Effects and 3D Frame are on, for the light on the bezel's wall: each level from a view of the one above, each texel the average of the box it stands for there, 2 × 2 texels where that level is even and a little over 2 where it is odd, weighted by how much of each the box covers. The chain from 416 × 287 is odd at most levels, and `generateMipmaps` neither keeps the light nor its place there: from level 4 on, a bar moving down the face moved in steps of up to 20 lines and its light varied by up to 50 %, so the wall's reflection of the bouncing bar in `scroller.bin` flickered. The CRT and flat passes draw straight into the drawable.
- **Phosphor:** only the white P4 of the black-and-white monitor the Juku shipped with; color presets were dropped as unhistorical. Color `#E8F0FF`, main decay 5 ms, afterglow decay 60 ms at weight 0.06. Videos of the МС6105 often show it as cyan; that is the camera's white balance, so the color stays as it is (MD §7.4).

- **Composite details:**
  - Beam σ runs linearly from 0.42 lines at Scanlines 0 to 0.15 at Scanlines 1, with bloom 0.15. At 0.42 the lines just touch and a solid white field is flat; at the default 0.68, σ ≈ 0.24, a solid white field dips about 35% between lines and a lone dark line between white ones stays dark. The first mapping, 0.55 − 0.33 × the setting, spent the left half of the slider on blurry beams and stopped at 0.22. Bloom was first tuned against video frames of a Juku, but their camera was overexposed and spread white into dark strokes: bloom 0.45 washed out one-line dark strokes in reverse-video text, and 0.3 left the darkest strokes at 33% of white. At 0.15 they reach 23%, close to the 17% of one-pixel black text on white in a photo of a similar monitor that was not overexposed.
  - The scanline profile fades to a flat field below about 3 drawable pixels per raster line, to avoid moiré in small windows.
  - Horizontally each of the five nearest canvas texels, read exactly with no bilinear filtering, lights a box one pixel wide whose edges are the CDF of a Gaussian spot of σ = 0.3 pixel (approximated by a logistic, which keeps an even field at exactly 1). A pixel of intensity I widens by 0.5 × bloom × I pixels on each side (0.075 at bloom 0.15), and the sum is capped at 1. A lone white pixel is therefore about 1.15 pixels wide, and a lone dark one between white pixels keeps about 85% of its darkness. The first version used five bilinear taps over ±1 pixel and was visibly blurry; the second, a normalized Gaussian gather over four texels, had no bloom across the line.
  - The halo is a 3 × 3 binomial blur with taps at ±2.5 texels and ±1.5 lines.
  - The beam is capped at 1.0 before tinting.
  - Vignette 0.25.
  - The glass is a rounded rectangle measured in half-heights of the tube, with corner radius 0.12, using an exact signed distance.
  - Frame: the Frame setting (0–1) × 4% of full white, in a neutral charcoal (linear 1 : 0.97 : 0.92). It is the bezel's front, drawn everywhere outside the glass, including the bars beside the tube in full screen.
  - 3D Frame (on by default) chooses between the bezel below and the flat frame of the first version: a recess next to the glass, lit by both the glass beside it and the room light on the frame. At the glass it is a quarter of (the face at the glass edge + the frame), and it eases out to the frame level as 1 − exp(−d / 0.02 half-heights). With 3D Frame off the tube keeps its full size in a window too.
  - Tube size, with 3D Frame on: in full screen the tube keeps the full height of the screen, so the bezel shows at the sides only. In a window, and in snapshot thumbnails, it is shrunk to leave 0.095 half-heights above and below it (0.13 at the sides), so the wall and a strip of the front show all round. The CRT-off path keeps the full size.
  - Bezel (MD §7.4), in half-heights of the tube: the front stands 0.11 (about 1 cm) in front of the edge of the glass. The inner wall leans out from the glass at 30° from the line of sight and turns into the front over a rounded lip of radius 0.04. The viewer is centered, 4.5 (about 50 cm) from the glass. A point of the cross-section at x outside the glass edge and z in front of it shows at e ≈ x + k z outside the glass outline, k = (p · n) / D, where `edge` is the signed distance to the outline and n its normal. That is linear on the straight part, and on the arc it inverts in closed form (−sin φ + k cos φ = const), giving the surface normal's angle φ from the line of sight: 60° on the wall, 0 on the front. The band is about 0.10 half-heights wide, matching photos of similar monitors; there is no edge between the wall and the front.
    - The face lights the bezel as a matte surface of reflectance 0.4 (0.5 looked a little glossy). A face point at distance d from the wall lights a square wall by d·h / (2 (d² + h²)^{3/2}) after integrating along it: half the hemisphere in all, most of it from within a few h. Five taps sit at the middles of five equal shares of that (d = 0.48, 1.02, 1.73, 3.18 and 9.95 h), each two reads ±0.44 r along the wall, from the phosphor mip level as wide as r = √(d² + h²), moved in by the point's own x. Each counts the lit phosphor and the unlit face, the latter with its darkening near the bezel. A surface tilted by φ gets 1 − cos φ of that.
    - h is measured from the phosphor, which lies behind about 1 cm of faceplate glass that looks two thirds as deep: 0.07 is added. Without it the foot of the wall saw only the darkened edge of the face and drew a hard dark line around the glass.
    - Room light: (1 + cos φ) / 2 of the Frame level, × (1 + 0.5 sin φ n_y), where n_y is 1 for the bottom wall, which faces up, and −1 for the top one, × (0.5 + 0.5 z / depth) for the shade nearer the glass. It is exactly the Frame level where the lip meets the front.
    - At the defaults, with a dark screen, the wall is between the front and the face (sRGB about 15–22 against 13 and 28). The white banner of the boot screen brings the top wall to about 44. The picture's borders keep lit pixels at least 0.16 half-heights from the glass edge, so the lower part of the wall sees them less.
    - History: the first version is the flat frame that 3D Frame off still draws. The second had a wall square to the glass meeting the front at a sharp edge, 0.03 half-heights wide; photos of similar monitors show a band about three times as wide and a rounded lip.
  - Background: the Background setting (0–1) × 8% of full white, in linear light, in a neutral gray with a slight green cast (linear 1 : 1.03 : 0.93, scaled to unit luminance). Brightness does not scale it, since room light does not depend on the monitor's knob. Video frames of a working Juku suggest 5–13%, but the camera brightened them: a 15% top looked unnatural in the app, and photos of similar tubes give 1–4%. It falls to 80% over the outer 0.08 half-heights of the glass, and is not used in the CRT-off path. The photos suggest 60% over about 0.15, but with a dark frame that read as heavy vignetting; it looked right only with a bright one.
  - No grain. The first version multiplied the picture by 3% noise that changed on every draw. It was removed when drawing stopped being continuous (§6.1): it would have frozen whenever the picture settled.
- **Settings:** `Settings.m` holds the `NSUserDefaults` keys and defaults: CRT on, curvature 0.022, scanlines 0.68, glow 0.085, background 0.175 (1.4% of white), frame 0.1 (0.4% of white), 3D Frame on, brightness 0.91. They were tuned against photos of a working Juku and of similar monitors, then set by eye in the app by someone who used the real machine. Users who changed a slider keep their saved values. `SettingsWindow.m` shows them in its Display pane (§6.7), which stays in sync with the View menu through `NSUserDefaultsDidChangeNotification`. Its Restore CRT Defaults button removes the saved values of the six CRT sliders (`crtSettingKeys()` in `Settings.m`), so the registered defaults apply again; it leaves CRT Effects and 3D Frame alone. The button is disabled while none of those keys is in the app's persistent domain.
- **View menu** (window tabbing is disabled, so AppKit adds no tab items):
  - CRT Effects (Cmd-E)
  - 1× to 3× in steps of 0.5× (the menu tags count half steps). At N× the tube is N points per raster line, 287 N pt tall, so 2N pixels per line on a retina display: 1.5× is 3, where the scanlines start to show (§7.3a). The titles use the decimal mark of the user's region (`NSNumberFormatter`), so 1,5× in Estonian. With CRT Effects and 3D Frame on, the window adds the bezel margin, giving 419 × 314, 628 × 471, 839 × 629, 1048 × 786 and 1257 × 943 pt; otherwise it is 383 × 287, 575 × 431, 765 × 574, 957 × 718 and 1148 × 861. The item whose size the tube is at (within a point) is checked, so none is after a resize by hand or after turning the bezel on or off; choosing it again resizes the window. In full screen the items are disabled, and so is a size whose window would not fit on the screen. The window keeps its title bar in place, moved up or across only to stay on the screen.
    - History: they were first 1×/2×/3× of 320 × 240 pt. The tube shows all 287 lines and, with the bezel, is shrunk by 1/1.095, so 1× was 1.5 pixels per line on a retina display and 0.76 on others, and smeared the text.
  - Enter Full Screen (Ctrl-Cmd-F)
- **Settings…:** Cmd-, in the app menu.
- **Verified in the app:**
  - CRT off gives sharp-bilinear pixels.
  - A routine typed into the monitor reprograms the timers to 48 bytes × 200 lines: hblank 16, hsync delay 4, vblank 113 and vsync delay 30, all in BCD. The image becomes 48 µs × 200 lines on the same ⅛ µs pixel grid, so its aspect is right. A reset restores 320 × 241.

### 7.4 Frame pacing
- Triple-buffered uniforms: up to three frames in flight, with a semaphore that waits until the GPU is done with a buffer before it is reused.
- Frames are drawn only while the picture can still change, at most one per Juku frame (§6.1).
- The pixel format is `BGRA8Unorm_sRGB` with the layer's colorspace set to sRGB.
- EDR is off.

---

## 8. Status

**Emulated:** the production board with RomBios (three system ROMs bundled, any 16 KB ROM file, expansion ROMs up to 32 KB), the 8080 to the cycle, the interrupt controller, the three timers and the two PPIs, the video modes and their timing, the keyboard with its DIP switches, the speaker, two double- or single-sided 5.25" drives with their sounds, the E4701 mouse, and snapshots of the whole machine. Both apps have the guest debugger (§12).

**Not emulated:** cassette tape and the early tape board, the local network (8251 networking and the F0–F3 coprocessor), printer output, the external 32 KB memory pages and the RAM disk, the 8" drive mode, and serial I/O. Where the machine description is unsure (MD §13), the emulator follows its best reading and says so.

---

## 9. Testing strategy

- **Unit tests per device:**
  - PIT: each mode, BCD wrap, the latch and read sequences. A randomized test programs, gates and reads three counters alike and clocks one pulse by pulse, one with `quiet`/`skip`/`clock` and one with `run`; they must agree in every field and in the pulses on which OUT changes.
  - PIC: priority, EOI, masking, CALL vector bytes.
  - PPI: BSR, mode word resets the outputs.
  - FDC: command state machines against a fake drive and the timing of DRQ and INTRQ.
  - Disk: geometry and offset mapping, write-track parsing, the dirty flag, and a blank disk's size and fill.
  - Keyboard: 74148 priority, column decode, DIP banks.
  - Mouse: byte format and direction bits.
  - Snapshots: CRC-32 and Adler-32 against known values, LZ77 round trips (random, all zeros, runs, the test images with `JUKU_TEST_DISKS`) and refused streams (a distance before the start, a length past the end, a stream short or long, a long varint), sections round trip, and every truncation and some bit flips are refused. PIT state round trips in every mode, binary and BCD, and runs on alike. FDC state round trips at every step of a seek with verify, read and write sector, read address, and read and write track, which brings up every command step.
- **Headless machine tests** (`core/tests/`):
  - ROM boot to the prompt (`core/tests/boot.rs`). The screen is read back as text by matching the ROM font against each 8×10 cell. RomBios draws each glyph's 9 rows starting on the cell's **second** line, and the header window is inverse video, so both polarities are matched. A cold boot reaches the `*` prompt after 2.2 s of emulated time and a warm reset after 0.6 s. The font is found by the glyphs for A and B, because it is at 1D02h in RomBios 3.43m and 1D01h in 3.43 #0031.
  - Frame interrupt period = 40 064 ± 0 cycles (`machine::tests`).
  - The raster (`video::tests`, `machine::tests`, `core/tests/raster.rs`):
    - In the raster alone: a line starts 16 µs after sync and the counter carries on to the next one; VER RTR low counts without drawing and its rising edge reloads; POF blanks while the counter runs; a gap in HOR RTR holds the counter; without sync the monitor runs on.
    - In the machine, with the RomBios timing on a ROM of NOPs: a write 2 µs after the beam fetched a byte shows only in the next frame, and one to a byte 8 µs ahead shows at once; a mode 0 control word to T1.1 in the middle of a line leaves an 11 µs gap, after which the counter carries on; VER RTR taken low and high in horizontal blank restarts the picture from D800h on that line, for one frame.
    - The demos, from the expansion window: `bounce.bin` moves its bar from line 47 down 201 lines and back in steps of at most 7, and a key gives the RomBios timing back; in 300 frames of `scroller.bin` the two lines of every text row are alike, where the per-frame copy split them in a quarter of the frames.
  - Caps Lock follower (`core/tests/caps_lock.rs`). With the host Caps Lock on from power-on, the cold boot ends with CONSW bit 0 set, and `a` echoes as `*A`. A warm reset clears CONSW and the follower sets it again, and turning the target off clears it.
  - Measured BLEEP frequency from the audio stream (`core/tests/monitor.rs`): a program poked into RAM calls BLEEP with A = 5 and DE = 08AEh, is started with `G0100`, and gives 900.1 Hz for 0.515 s. The same file types `D0000001F` and checks the dump. Typing goes through `Machine::set_key` with 60 ms holds.
  - ZOO.COM from the games disk, `jukgame1.juk` (`core/tests/zoo.rs`): its title screen keeps the RomBios mode, and its demo reprograms PIT #1 ch1/ch2 to 16 and 4 µs and PIT #2 ch1/ch2 to 112 and 45 lines, giving a **48 × 201** byte mode (384 × 201 pixels, porches 12 µs and 67 lines). The test checks the geometry and that pixels are drawn past column 352.
  - T3.2's clock (`core/tests/t32_clock.rs`). On both RomBios ROMs, TIMCNT (D463h) goes up by 50 in a second on the frame clock, by 24 500–24 655 a frame with the option on, and by 1 a frame again once it is off. With `JUKU_TEST_DISKS`, RDEM.COM from the programs disk draws no pixel below the JUKU logo in the 8 s after it has started on the frame clock, and at least 40 in 30 columns on 1.2308 MHz. Like the ZOO test, it waits for the program to clear the screen (`common::start_program`), since loading takes 12–14 s.
  - Key click (`core/tests/key_click.rs`). With the patch on, a cold boot still passes the ROM test and reaches the prompt, and a key press at the prompt writes nothing to port 19h. Turning the click back on brings the port 19h writes back. A unit test checks that a ROM without the click code is not changed. Another runs over both RomBios ROMs (`common::ROMS`): the patch is found, the chip sum still matches, and the click goes off and on.
  - JBASIC (`core/tests/expansion.rs`). On both RomBios ROMs, `jbasic11.bin` is plugged in, `A` at the prompt reaches BASIC's `READY`, and `PRINT 6*7` gives 42. Without an expansion ROM, `A` returns to the monitor.
- `Machine::log_io(true)` records every OUT with its cycle, for tracing device programming from tests or examples.
  - Snapshots (`core/tests/snapshot.rs`). A machine at the prompt, with a key held, is saved and restored into a machine running another ROM with the key click off. The restored one saves the same file, and after typing a monitor command both are in the same state, compared as their snapshots. The same holds when the snapshot is taken in the middle of an EKDOS sector read (`JUKU_TEST_DISKS`). The file is a 320 × 241 PNG, version 1 and 2 files are refused as older, disks come back clean, host sections come back and capital host tags are refused, the ROM is stored unpatched with the click flag, and an expansion ROM is stored without its trailing FFh and removed by a snapshot without one. Bad files (not a PNG, truncated, bit flips, a newer version, a bad memory or PIT mode, a short or extra section, a missing one) are refused and leave the machine as it was.
  - Disk boot (`core/tests/disk.rs`). Both disk images boot EKDOS 2.30 through `T`, `D`, `D` and list the directory. On the programs disk the test then runs `SAVE 2 ZZ.COM`, ejects, re-inserts the image into a fresh machine, boots again and finds `ZZ.COM`. With a blank disk in B:, `DIR B:` says NO FILE, and after `SAVE 1 B:NEW.COM` it lists `NEW.COM`.
  - Drive sounds (`drive_sound::tests`, `core/tests/drive_sounds.rs`). The synthesis alone, at 44.1 and 48 kHz: the levels of a disk swapped in, the spinning motor, a 40-step seek at 10 ms and an eject stay in range, all is quiet and every voice and tail let go once the motor has stopped and the disk is out, and turning the sounds off fades them and leaves the disks' state as the cut-off sounds would have. Half the volume halves the output and none silences it; a step of B: is 1.5 times as loud on the right as on the left, and the two channels' powers add up to the middle's; with the idle motor muted, a spinning disk is heard for 10 s, is 18–30 dB down a second into the fade, is quiet after it, and is heard again after a command. In the machine, two alike, one with the sounds on and one off, pull their audio as they run: a disk put in before the machine runs is silent, one put in A: is heard 1.4–1.6 times as loud on the left, one in B: on the right, one put in and taken out later is heard, nothing is added once the sounds are turned off, and both machines end in the same state, compared as their snapshots. With `JUKU_TEST_DISKS`, the same holds for an EKDOS boot, `DIR` and an eject, and with `JUKU_SOUND_WAV` set to a path that run is written there as a WAV file, to listen to. With the idle motor muted, the drive is heard spinning at the `A>` prompt, is quiet 13 s later, and is heard again after `DIR`.
  - The debugger (`debug::tests`, `cpu::disasm::tests`, `core/tests/debug.rs`, §12.4). On a test bus: breakpoints stop before their instruction and not again on going on; pause; the shadow stack through calls, returns and a return address popped; steps by instruction, over a call (and a breakpoint inside it), out of nested calls and with no frames, by range over and into a call; an interrupt during a step run through, or stopped in with `step_into_interrupts`; a step over HLT ending after the interrupt, and no breakpoint while halted; watchpoints on writes, reads (not the instruction's own bytes, nor an interrupt's) and ports. In the machine: debugging with a breakpoint and watchpoints nothing reaches leaves the machine in the state a plain one reaches, compared as snapshots, at the prompt and, with `JUKU_TEST_DISKS`, after booting EKDOS and `DIR`; a stop runs nothing and the time asked for meanwhile is not made up; a breakpoint at CONSTAT stops there; at the prompt, steps from CALL to CALL and over each one end after the CALL with SP as it was, also when a frame began in the call, and no instruction step ends in a handler. The disassembler agrees with juku-sdk's instruction table on all 256 opcodes (the debug server's `cpu_disassembler_is_the_sdks`).
  - **Disk images in tests:** the images are not part of the repository. The disk, drive sound, mouse, ZOO and RDEM tests read `jukprog1.juk` and `jukgame1.juk`, and the JBASIC test reads `jbasic11.bin`, from the directory named by `JUKU_TEST_DISKS` (`common::test_file`) and are skipped when it is not set. A relative path is resolved from `core/`. The tests work on the copy in memory, so the files never change.
- **Build prerequisite:** the Xcode "Metal Toolchain" component (`xcodebuild -downloadComponent MetalToolchain`), which provides `xcrun metal`.
- **Golden screenshots:** `cargo run -p juku --profile test --example boot_dump -- <seconds> <out.pbm>` boots the ROM and writes the frame's active area as PBM, for visual checks while developing. (The release profile cannot be used from cargo directly: its `panic = "immediate-abort"` needs `build-std`.)
- **Cross-check:** MAME's `juku` driver is the behavioral reference for anything ambiguous. MD §13 lists the known differences: PA6, the PC5 select decode and T3.2, whose clock is an option here (§4.5).

---

## 10. Decisions

| Topic | Default in this spec |
|---|---|
| Bundle id | `io.github.sinijarv-juku.juku-emu` |
| Minimum macOS | 14.0, since before publishing (it was 26.0): the `NSView` display link needs macOS 14; the rest, macOS 11 (`UTType`, the toolbar style, SF Symbols). `build.sh` sets `MACOSX_DEPLOYMENT_TARGET` for Rust, clang, `metal` and `actool`. On 14 and 15 the app shows `Juku.icns`, and AppKit's look of those versions |
| ROM delivery | ROM files inside the app bundle, from `resources/roms`; three 16 KB images, `ekta37.bin` (RomBios 3.43m) by default, or a 16 KB file of the user's, chosen in Settings or Machine → System ROM. Expansion ROMs are not bundled: the user picks a file |
| Emulation thread | the main thread, paced by the display link and trimmed by the audio clock |
| Sandbox | off for v1 (the drives and Recent Images reopen files through bookmarks) |
| Disk files | the disk is in memory; its file is written only when the user saves it, through a save panel (§6.3) |
| UI language switch | on the next launch, through the per-app `AppleLanguages` key; switching live would mean rebuilding every menu and window, and a forced restart would lose the running program |
| CPU test binaries | downloaded by the tests from superzazu/8080 at a fixed commit, into `target/cpu_tests/`; skipped when they cannot be |

---

## 11. Windows front-end

Status: the app boots to the `*` prompt with the CRT picture, takes the keyboard and the mouse, plays the speaker, has Pause and Warp, runs EKDOS from disk images it writes back, has the View menu and full screen, the Settings window and the ROM menus, saves and opens snapshots, registers its file types with Explorer, with thumbnails of snapshots, and serves the guest debugger (§11.8). Each part of this section gives the plan, then "as built" notes for where the implementation adds to it or differs. Keeping the disks in memory (§6.3) and the build changes since were checked on a Mac only (`cargo check --target x86_64-pc-windows-msvc`), not yet run on Windows.

The macOS app (§6, §7) is the reference for behavior. The Windows app has the same menus, settings, pacing, drawing rules, disk and snapshot handling, and this section records only how it does them on Windows and where it has to differ.

### 11.1 Language and build
- **Rust.** A crate of its own in `frontends/windows/`, building `Juku.exe`. It depends on the core by path (`juku = { path = "../../core" }`) and calls its Rust API (`Machine` and the rest) directly, as the core's integration tests do; the C ABI (§5) is the Mac app's boundary only. It was first a crate of its own, outside a workspace, with a copy of the core's release profile (`panic = "immediate-abort"`, LTO, one codegen unit) and its own `Cargo.lock`.
  - *As built:* the crates are one Cargo workspace (§3.1), which holds the profiles and the lock file; `build.cmd` builds `-p juku-windows` and `-p juku-thumbnail`, one after the other, so that the DLL is built with its own features of the `windows` crate only.
- **One dependency:** Microsoft's `windows` crate, for Win32, COM, Direct3D 12, DXGI, WASAPI, WIC and BCrypt. It is pinned to a version, with only the features the app uses. The core keeps no dependencies. (As built: `windows-core`, which the `windows` crate is built on, is named as a dependency too, at the same version, because the `#[implement]` macro for the app's own COM objects refers to it by name.)
- **Target** `x86_64-pc-windows-msvc`, with `-Z build-std=std` and `-C target-feature=+crt-static`, so the exe needs no Visual C++ runtime installed.
  - **The Universal CRT is Windows'.** The static C runtime is in two parts: the small VC runtime (startup, `memcpy` and the like) and the Universal CRT (math, strings, heap, stdio), which has been part of Windows since Windows 10. `build.cmd` links the first in and takes the second from Windows' `ucrtbase.dll`, through its `api-ms-win-crt-*` API sets: `/NODEFAULTLIB:libucrt.lib` and `ucrt.lib` as link arguments. The exe went from 737 KB to 634 KB, the thumbnail DLL from 114 KB to 31 KB. It still needs no runtime installed on Windows 10 and 11, the only systems the manifest names. Leaving out the VC runtime too would mean running std's startup and thread-local storage without it, which std does not support, for its small share of the rest. Linking needs the Visual Studio Build Tools (the MSVC linker and C runtime) and the Windows SDK; rustc finds `link.exe` itself, so no developer shell is needed. There is no C or C++ code. ARM64 comes later, as a `dist` option.
  - Checked before the port: with these flags the core builds as a static library on Windows and needs only `kernel32 ntdll userenv ws2_32 dbghelp` and `libcmt`; `cargo test` passes there.
- **Embedded data.** The ROMs (`resources/roms`), the strings (`resources/strings`) and the compiled shaders go into the exe with `include_bytes!`, so `Juku.exe` is the whole app. Only the icon, the version information and the manifest are Windows resources: `build.rs` runs `rc.exe` from the newest installed Windows SDK on `frontends/windows/resources/Juku.rc` and hands the `.res` file to the linker. The version is read from `frontends/macos/resources/Info.plist`, so both apps have one version.
- **Manifest:** per-monitor DPI awareness (v2), common controls 6, UTF-8 as the process code page, `supportedOS` Windows 10 and 11.
- **Shaders.** `Shaders.hlsl` holds the passes of §7.2, ported from `Shaders.metal` with the same constants. `build.rs` compiles it with `d3dcompiler_47.dll`, which is part of Windows, to shader model 5.1, which Direct3D 12 runs; `dxc` is not needed. The CRT uniforms are a `#[repr(C)]` struct laid out by HLSL constant buffer rules.
- **`build.cmd`** at the root is a plain batch file. It runs cargo with the target, `build-std` and the flags, then copies `Juku.exe` and `Juku.pdb` to `target\windows\`. Its modes are those of `build.sh`: the default (tuned for this CPU, `-C target-cpu=native`), `debug` and `dist`. `dist` also makes `target\Juku-<version>-windows-x64.zip`. The flags are not kept in `.cargo/config.toml`, which would give them to `cargo test` too (§3.1). The cargo output shares `target/cargo` with the Mac build; the target triples keep them apart. The file has no labels, so it also runs with the LF line endings Git stores (`core.autocrlf=input` keeps them).
  - **As built:** `build.cmd` builds the thumbnail handler's crate as well and copies `JukuThumbnail.dll` and its `.pdb` beside `Juku.exe`. Explorer keeps the DLL loaded for a while after it has drawn thumbnails, so the copy can fail until it lets go; the script says so. `dist` reads the version from `Info.plist` with a PowerShell one-liner and packs `Juku.exe` and `JukuThumbnail.dll` with Windows' own `tar.exe`, which writes zip files. In this build the exe was 685 KB, the DLL 114 KB and the zip 408 KB; with the Universal CRT taken from Windows (above) the exe is 634 KB and the DLL 31 KB.
  - **Icon.** `Juku.ico` is made from the Mac's Icon Composer bundle by `resources/make_artwork.ps1`, with what Windows 11 has: headless Microsoft Edge draws `icon.json`'s gradient and the two SVG layers in a rounded square (20% corners, 1/32 of the size inside the edges, transparent outside) at 16, 20, 24, 32, 40, 48, 64, 96, 128 and 256 px, and the script packs the PNG images into the `.ico`. macOS masks its icon itself; on Windows the shape is part of the picture. The `.ico` is committed, and made again when the Mac icon changes. `Juku.rc` makes it the exe's first icon: Explorer, the taskbar, the window's title bar (its class icons) and About show it.
  - **As built:** release builds keep line tables (`debug = "line-tables-only"`), which go to `Juku.pdb` for reading crash dumps; the exe has no symbols, like the Mac's stripped one. The release exe is 241 KB. `Cargo.lock` is committed, and the `windows` crate is pinned to 0.62.2. With the workspace (above), `build.cmd` sets the line tables itself (`CARGO_PROFILE_RELEASE_DEBUG`), so that the Mac's library is built without them.
- **Modules** follow the Mac files, so the two can be read side by side: `main.rs` (`Main.m`), `emu_window.rs` (`EmuView.m`), `renderer.rs`, `audio.rs`, `keymap.rs`, `disks.rs`, `recent_disks.rs`, `settings.rs`, `settings_window.rs`, `roms.rs`, `snapshots.rs`, `keyboard_help.rs` and `strings.rs`; `file_dialogs.rs` holds the open and save dialogs. The Explorer thumbnail handler is a second crate, `frontends/windows/thumbnail/`, a DLL (§11.6).
- **Logging:** `log!` writes to `OutputDebugStringW`, which a debugger or DebugView shows. Debug builds are console programs (`windows_subsystem` is set for release only), so started from a terminal their log shows there, as `run.sh` shows the Mac's.
- **Shaders as built.** All five entry points are compiled, with strictness on and warnings as errors, though at first only decode, persistence and the flat pass were drawn. Porting notes:
  - `line`, `flat` and `out` are HLSL keywords, so those variables are `row`, `flatField` and `outside`.
  - fxc reports a use of an uninitialized value after an early return in a function (`texelAt`, `bezel`), so those return one result variable instead.
  - Decode reads the core's raster from a `Buffer<uint>` (`R8_UINT`) in an upload heap, 64 bytes a line, rather than from a texture: no copy and no texture state to track. Each draw in flight has a buffer of its own.
  - Every pass sees the same root signature: the 36 words of `CRTUniforms` as root constants (b0), a table of three views (t0 and t1 textures, t2 the bytes; null views where a pass needs none), and two static samplers: bilinear at level 0 (Metal's `linearClamp`) and trilinear (`mipClamp`), both black outside.
  - The D3D debug layer is turned on in debug builds when Windows' optional Graphics Tools are installed, and its messages go to the log after every draw.

### 11.2 How the Mac pieces map

| macOS | Windows |
|---|---|
| Display link, 20 ms `NSTimer` fallback (§6.1) | A high-resolution waitable timer at the display's refresh rate, waited on with `MsgWaitForMultipleObjectsEx` in the main loop (§11.5). A pacer thread keeps the machine running, and drawing, at the same rate while Windows runs a modal loop (moving or resizing the window, an open menu, a dialog). |
| Metal (§7) | Direct3D 12 (§11.5) |
| CoreAudio unit (§6.6) | WASAPI shared mode, event-driven, on a thread of its own registered with MMCSS; 48 kHz stereo float, the ring's two channels interleaved, from `AudioRing::pull` |
| `kVK_*` table (§6.4) | Scan codes with the extended-key bit, which are positional too (§11.3) |
| Mouse capture (§6.5) | Raw input (`WM_INPUT`) for the movement, `ClipCursor` to the window, the cursor hidden |
| Native full screen | A borderless window covering the monitor, without the menu bar (F10 opens the menus), with the cursor rules of §6.2 (§11.5) |
| `NSOpenPanel` / `NSSavePanel` | `IFileOpenDialog` / `IFileSaveDialog` |
| `NSAlert` | `TaskDialogIndirect` |
| Settings window, Keyboard Mapping, About | Windows built in code, as the Mac app uses no nibs: no dialog templates. Segoe UI from the system metrics, scaled per monitor. |
| `NSUserDefaults` | `Juku.ini` (§11.4) |
| Bookmarks | Path plus file ID (§11.4) |
| `mac ` snapshot section | `win ` section (§11.4) |
| Title and subtitle | Windows has no subtitle, so the title carries both: `Juku — A● — Paused`. The drive letters stay lit 0.3 s after a command, as on the Mac; the title is set only when its text changes |
| Dock recent documents | Recent Images plus the taskbar jump list (`SHAddToRecentDocs`) |
| Finder Open With, one app instance | One instance: a named mutex, and a second start passes its files to the first with `WM_COPYDATA` and exits. Without it, double-clicking an image would start a second app writing the same INI file and disk images. |
| Quick Look extension | Explorer thumbnail handler (§11.6) |
| Drag and drop, Option for B: | `WM_DROPFILES` (`DragAcceptFiles`), Alt held for B: (§11.4) |
| `.dmg` | `.zip`, and an installer (§11.7) |

- **Mouse as built** (`emu_window.rs`, as §6.5):
  - **Capture.** Alt+M (Input → Capture Mouse), or a click in the window while the running program has IR6 unmasked. The click that activates the window does not count, as on the Mac (`acceptsFirstMouse`). Capturing registers for raw mouse input (`RegisterRawInputDevices`, usage page 1, usage 2, for this window), hides the cursor (`WM_SETCURSOR`) and holds it in the middle of the window with `ClipCursor` on a one-pixel rectangle, put there again when the window moves or changes size. It is shown there on release.
  - **Movement** comes from `WM_INPUT`: relative counts as the mouse reports them, before Windows' pointer speed and acceleration. Mouse Speed (`MouseSensitivity`, default 0.5) is Juku counts per mouse count, the remainder kept as a fraction, as the Mac does per point. A mouse that reports absolute positions (remote desktop, a virtual machine) is turned into movement in screen pixels. The buttons come from the same input: left to left, right to right.
  - **Release:** Alt+M, focus lost (Alt+Tab, another window, Settings), a menu opened (F10, Alt+Space) and the window moved or sized. While captured the title reads `Juku — Press Alt+M to release the mouse`.
  - **Verified** with `gtr.com` from the programs disk: a click on its canvas captured the mouse; 80 × 60 counts of injected relative movement moved its cursor 40 × 30 pixels, and a drag of 100 counts with the left button drew a line 50 pixels long. A click at the monitor prompt did not capture; Alt+M did, and Alt+, then released it.
- **Sound as built** (`audio.rs`):
  - The ring is made as on the Mac: 48 kHz, primed with 2048 samples (about 43 ms) after an underrun. The rate trim of §6.1 follows its fill, and only when an output was opened; without one the machine runs on the host clock.
  - A thread of its own opens the stream, so COM there is in the multithreaded apartment and the main thread stays free for the single-threaded one the file dialogs need. It reports to `AudioOutput::start` whether it could open one. It registers with MMCSS as "Pro Audio".
  - The stream: shared mode, event-driven, a 20 ms buffer, 48 kHz stereo float with `AUTOCONVERTPCM` and `SRC_DEFAULT_QUALITY`, so Windows converts to the device's own format. Each event fills the free part of the buffer from the ring, interleaving its left and right channels, and marks the buffer silent when the ring had nothing.
  - **The default device is followed**, as the Mac's default output unit does: an `IMMNotificationClient` signals a change of the default output, and the stream is opened again on the new one. Any error from the stream (the device unplugged) also reopens it; without a device it tries again every second. Not yet tried with a second output device.
  - **Pause and Warp** behave as on the Mac (§6.2): paused, the machine does not run and the ring runs dry; warp mutes the core (`set_audio_muted`), runs slices of 20 000 cycles up to 20× within 75% of the tick and at most 15 ms, and measures its speed every 0.5 s. Alt+P and Alt+W; their menu items are ticked as the menu opens (`WM_INITMENUPOPUP`, the Mac's `validateMenuItem`).
  - **Verified:** a loopback recording of the default output while typing at the prompt holds one burst per key, 26 ms at 2005–2007 Hz: the RomBios click is 2 kHz (MD §14), and the trim, which may run up to 0.5% fast while the ring fills after start, accounts for the rest. The title read `Juku — Paused` while paused and `Juku — Warp Speed (20×)` in warp.

### 11.3 Keyboard and menus
- **Left Alt does what Cmd does**, with the same letters: Alt+R Reset, Alt+Shift+R Power Cycle, Alt+O, Alt+S, Alt+Shift+S and Alt+L for snapshots, Alt+P Pause, Alt+W Warp, Alt+1/Alt+2 Insert, Alt+Shift+1/2 Eject, Alt+E CRT Effects, Alt+M Capture Mouse, Alt+, Settings. Full screen is Alt+Enter and Exit Alt+F4, the Windows conventions. Ctrl cannot take Cmd's place, because it is the Juku's CTRL key.
- Events with Alt held never reach the guest. Alt pressed and released alone does not open the menu bar, since the next keys would then go to the menu instead of the guest; F10 does (the Juku has F1–F8 only).
- **Right Alt is `; +`**, as Right Option is on the Mac. On layouts with AltGr, Windows sends a Left Ctrl with Right Alt; it is filtered out.
- **Underlined letters (mnemonics):** inside the menus, chosen freely. On the menu bar they must avoid the Alt shortcut letters of each language: Alt+M is Capture Mouse, so Machine cannot use M. They are in Windows-only string keys such as `"&Machine" = "M&asin";`, since the shared keys are the Mac titles.
  - **As built:** each menu title is looked up as `&` plus the Mac's English title, which both `.strings` files hold with the letter marked (`"&Machine" = "Ma&chine";` in English); a title without such a key has no letter underlined (`strings.rs`, `menu_title`). The menu bar is Ma&chine, &Disks, &Input, &View and &Juku in English, M&asin, &Kettad, Sisen&d, &Vaade and &Juku in Estonian, none of them a shortcut letter (R P W E M , and O S L for the snapshots). File names in menus have their `&` doubled.
  - **Left Alt with a letter of the menu bar opens that menu**, as Windows apps do: the `WM_SYSCHAR` goes on to `DefWindowProcW` only for such a letter with Left Alt held, outside full screen and when the key was not a shortcut. Any other Alt+letter is dropped as before, so nothing beeps, and Right Alt with a letter opens nothing, since Right Alt is a Juku key. F10 then a letter works too.
- The table is as in §6.4, keyed by scan code. Key repeats (bit 30 of `lParam`) are ignored. The guest Caps Lock follows `GetKeyState(VK_CAPITAL)`, read on every tick. Unlike the Mac's, it follows only while the window has the keyboard: Windows updates a thread's view of the lock state only then.
- **As built:**
  - The shortcuts are looked up in the window procedure (`main.rs`, `SHORTCUTS`), not through an accelerator table, because `TranslateAccelerator` takes either Alt key, and Right Alt is a Juku key. A shortcut is matched by its virtual-key code, so it follows the layout's letters, as Cmd shortcuts do on the Mac. A key up always reaches the Juku, with or without Alt, so none is left held.
  - With Left Alt held, the other keys still come as `WM_SYSKEYDOWN` and go on to `DefWindowProcW` for Alt+F4 and Alt+Tab; the `WM_SYSCHAR` that follows is dropped, except Alt+Space for the window menu, since Windows beeps for an Alt+letter that is no menu's. Alt pressed alone ends in `SC_KEYMENU` with no key, which is dropped unless F10 was pressed.
  - The AltGr Left Ctrl is recognized by the message that follows it: a Right Alt down with the same message time. Its key up is dropped with it. The Shift that Windows adds around keypad and cursor keys with Num Lock on comes with the extended-key flag (`E0 2A`) and is ignored.
  - Focus loss and opening a menu release every key, as on the Mac. On focus gain Left Alt's state is read again, since its key up may have gone to another window after Alt+Tab.
- **Menus:** the same menus in the same order as the Mac's (`Main.m`): Juku (About Juku, Settings…, Exit), Machine, Disks, Input and View. The Mac-only items (Hide, Show All, the Window menu) are left out.
  - **As built:** Juku has About Juku, Settings… (Alt+,) and Exit; Machine has Reset, Power Cycle, System ROM, Expansion ROM, Pause and Warp Speed, and the snapshot items (§11.4); Input has Capture Mouse (Alt+M), ticked while captured, and Keyboard Mapping. The ROM submenus are built as they open, like Recent Images: System ROM lists the bundled ROMs by title ("RomBios 3.43m #0037 (Default)"), the user's last ROM file by its name and Other…, with the ROM in the machine ticked; Expansion ROM lists None, the last file and Other…. Windows menus have no tooltips, so the file's path is not shown there; the Settings lists show it.
  - **About Juku** is a task dialog with the name, "Version" and the version (from `VERSION`, §3.3) and the copyright, which is what the Mac's standard About panel shows. Its icon is the app's.
  - **Settings and Keyboard Mapping** are windows of their own, owned by the emulator window, so they stay above it, in full screen too. They are not dialogs, but the message loop passes their messages through `IsDialogMessageW`, so Tab moves between controls and Esc closes them. Left Alt shortcuts pressed in them go to the menus, as the Mac's menu bar serves every window, except Alt+W, which closes them as Cmd-W does on the Mac. Closing hides them, so they open again where they were.
  - **As built:** the Disks menu is the Mac's: a submenu per drive, titled `&A: prog.juk` and so on with the drive letter underlined, holding Insert… (Alt+1, Alt+2), New Disk, Recent Images, Save Disk…, Eject (Alt+Shift+1, Alt+Shift+2) and Write Protect. The Eject and Write Protect items are disabled for an empty drive, and Write Protect is ticked as on the Mac; all are brought up to date in `WM_INITMENUPOPUP`, where the drives' titles are set too (`ModifyMenuW`), with the Mac's dot (`DiskDrives::label`), and the status line at the top of each drive's submenu (`DiskDrives::status`). Windows menus cannot hide an item, so the status line and its separator are put in and taken out (`set_disk_status`). The Insert dialog is titled "Insert in A:" or "Insert in B:", Windows-only strings.
  - **Commands run outside the window's state.** A menu command or shortcut (`main.rs`, `command`) takes the window's state only for the moments it needs it, never while a dialog or an alert is open. A dialog's modal loop still delivers the pacer's ticks (§11.5), so the machine keeps running and drawing behind it, as behind a sheet on the Mac. Checked: with warp on, the title went on showing the measured speed while the Insert dialog was open.

### 11.4 Files and settings
- **Folder:** `%LOCALAPPDATA%\Juku\` holds `Juku.ini`, `Quick Save.jukusnap`, `ROMs\` (the ROMs kept for snapshots, §6.8) and `Snapshots\`. Save Snapshot… opens in `Snapshots\` the first time, then in the last folder used. AppData is a hidden folder, so it is not where users browse to in Explorer. The Mac app is unchanged.
- **`Juku.ini`:** UTF-8, read and written by a small parser of the app's own, because the Windows profile functions handle UTF-8 poorly and paths may hold ä, õ. The keys have the names of the Mac defaults (§6.7); binary values (the snapshot key, file IDs) are hex, and lists (Recent Images, the drives) are numbered keys in a section of their own. It is written when a setting changes, to a temporary file that then replaces it.
- **Bookmarks.** A file is remembered by its path, its volume serial number and its 128-bit NTFS file ID. When the path no longer finds a file, `OpenFileById` finds it wherever it was moved or renamed on the same volume, as a bookmark does.
  - **As built:** the path is tried first, not the ID: saving replaces an image file with a new one (atomically), which has a new ID, so the old ID no longer names it. The drives are saved again after every save, with the new ID. A file whose path now holds another file is therefore taken at its path. The ID is found again through a handle on the root of the old path's drive, so a file on a drive letter that has changed is not found.
  - **`Juku.ini` as built:** `[Drives]` holds `A`, `A.FileID` and `A.WriteProtected` for each drive whose disk has a file (`B` likewise), and `[RecentDisks]` holds `1` … `10` with `1.FileID` and so on. A file ID is written `volume:id`, both in hex. A drive with no entry is empty. The file is created on the first change; nothing is written before.
- **Disks** (`disks.rs`, as `Disks.m`, §6.3):
  - **Insert:** `IFileOpenDialog` titled "Insert in A:", filtered to Disk Images (`*.juk;*.img;*.dsk;*.cpm`).
  - **Saving:** Save Disk… opens `IFileSaveDialog` with Disk Images, the disk's folder and name (`SetFolder`, `SetFileName`; Documents for a disk without a file) and `FOS_OVERWRITEPROMPT`. The image is written to `<name>.juku-tmp` beside it, which then replaces it with `ReplaceFileW`, keeping the old file's attributes and permissions; `rename` is the fallback.
  - **The save alert** (`disks::may_remove`) is a `TaskDialogIndirect` owned by the window, with the Mac's message and note, the system's question mark (task dialogs have none of their own, so `IDI_QUESTION` from `LoadIconMetric` with `TDF_USE_HICON_MAIN`), and the buttons Save…, Don't Save and Cancel, all three its own so they are in the app's language; Cancel has the ID `IDCANCEL`, and with `TDF_ALLOW_DIALOG_CANCELLATION` Esc cancels. It comes before a changed disk leaves its drive, as on the Mac. On quit (`WM_CLOSE`) the changed disks are asked about before the window goes, and Cancel keeps it open. Its message names the disk and the drive through `strings::format`, which takes NSString's positional `%1$@` and `%2$@` as well as `%@`, since the Estonian text puts the drive first. The alert and the save dialog run with the window's state let go, as every dialog does (§11.3). While either is open, a file from Explorer or another start is ignored, as it was while the import window was open.
  - **Log off and shut down.** With a changed disk, `WM_QUERYENDSESSION` returns FALSE after `ShutdownBlockReasonCreate` ("Disks have unsaved changes.", a Windows-only string) and posts the window a close, so the alerts come up while Windows shows the reason; `WM_CLOSE` destroys the reason once they are answered. If the user shuts down anyway (`WM_ENDSESSION`), the changes are lost.
  - **Restore at launch:** before the machine runs; a missing or bad image is reported once the window is up, and its drive stays empty. Restoring does not touch Recent Images.
  - **Recent Images** is built as the submenu opens: one item per image that can still be found, inserting it in the submenu's drive, and Clear Menu. The items of B:'s list have command IDs 100 above A:'s. Images also go to the jump list (`SHAddToRecentDocs`), which shows them once the file types are registered. Clear Menu leaves the jump list alone: Windows can clear only every app's recent documents at once.
  - **Drop:** `DragAcceptFiles` and `WM_DROPFILES`, which need no COM object of the app's own: the first file goes in A:, or in B: with Alt held as it is dropped. Not checked with a real drag yet.
  - **One instance:** the mutex `Local\io.github.sinijarv-juku.juku-emu` marks a running Juku. A second start finds its window by class, sends it the file named on its command line with `WM_COPYDATA`, brings it to the front and exits. The file is opened from a posted message, so the second start does not wait on an alert. A file named on the first start's command line goes into A: once the window is up, replacing the restored disk (the Mac's Open With).
  - **Alerts** are `TaskDialog`s with the message as the main instruction, the error icon (`TD_ERROR_ICON`) and an OK button. Every alert but the save alert says that something could not be done: a file that could not be read or saved, or was not what it should be, file types that could not be registered. The Mac's `NSAlert` shows the app icon instead, as macOS alerts do; About Juku shows it on Windows too. The fatal startup errors are `MessageBoxW`s with `MB_ICONERROR`, since they come before the window.
  - **Verified**, while disks were still written back to their files, with the test images (`floppies/`), on copies:
    - `Juku.ini` naming a copy of `jukprog1.juk` in A: with Write Protect off: the drive comes back at launch, `T`, `D`, `D` boots EKDOS with A● lit in the title, `DIR` lists the disk, and `SAVE 2 ZZ.COM` is written back to the file within the write-back interval: the file changed and holds the `ZZ      COM` entry. `A.FileID` changed with it.
    - The image moved to another folder and renamed: the next launch found it by its ID and wrote the new path to `Juku.ini`.
    - A second start with `jukgame1.juk` on its command line: it exited, and the running Juku had the image in A:, read-only, at the top of Recent Images.
    - Alt+Shift+1 emptied A:; Recent Images, reached with the keyboard, put the image back.
- **`win ` section.** As the `mac ` section of §6.8: the files on this computer, encrypted with AES-256-CBC and authenticated with HMAC-SHA256, both from BCrypt, which is part of Windows. The 64-byte key is `SnapshotKey` in the INI file. Not DPAPI: its output carries the GUID of the user's master key, an identifier that would link a user's snapshots to one another, which §6.8 rules out. A Mac snapshot opened on Windows has no `win ` section and is restored as one from another computer, and the other way round.
- **Snapshots as built** (`snapshots.rs`, as §6.8):
  - **Menus** (Machine, between the ROMs and Pause, as on the Mac): Open Snapshot… (Alt+O), Save Snapshot… (Alt+Shift+S), Quick Save (Alt+S) and Quick Load (Alt+L). Quick Save and Quick Load use `%LOCALAPPDATA%\Juku\Quick Save.jukusnap`, and Quick Load is disabled until it exists; both show "Snapshot Saved" or "Snapshot Loaded" in the title for 2 s. Save Snapshot… proposes `Juku <date time>.jukusnap`, opens in `Snapshots\` until the user has saved elsewhere (`SetDefaultFolder`: Windows then remembers the folder), and writes the file as disk images are written (§11.4). The open dialogs show "Juku Snapshots" (`*.jukusnap`). A snapshot from Explorer, the command line, another start of Juku or a drop is opened rather than inserted. Saved and opened snapshots go to the jump list.
  - **`meta`** is the Mac's, key for key; `app` is `Juku <version>` and `saved` is UTC.
  - **`win ` section.** The files the snapshot came from, as `key=value` lines: `disk0` and `disk1`, the paths of the disks' files, `SystemROM` and `ExpansionROM` for ROM files. It is sealed as the Mac's `mac ` section is (§6.8): `version (1), IV, ciphertext, MAC`, AES-256-CBC with PKCS#7 padding and HMAC-SHA256 over the rest, from BCrypt's built-in algorithm handles (`BCRYPT_AES_CBC_ALG_HANDLE`, `BCRYPT_HMAC_SHA256_ALG_HANDLE`), with random bytes from `BCryptGenRandom`. The 64-byte key is `SnapshotKey` in `[Settings]`, in hex, made on first use. The MAC is compared in constant time. A section that does not check out is ignored and the snapshot is restored as one from another computer; a Mac snapshot has none at all.
  - **Opening** is the Mac's (§6.8): the changed disks are asked about (§6.3), the snapshot is loaded, its ROMs are matched by their bytes (a bundled ROM, the file it came from, the ROM settings, the files kept in `%LOCALAPPDATA%\Juku\ROMs`; a ROM found nowhere is written there under its `meta` name, with the characters Windows does not allow in names replaced), the record of the ROMs in the machine is set before the settings are applied so nothing restarts, and the disks take their files from `win `.
  - **Verified** (a copy of `jukprog1.juk`; screenshots of Juku's windows only):
    - At EKDOS during `DIR`, Alt+S saved a 1 MB snapshot whose picture is the CRT screen at 640 × 480, and whose bytes hold no path or user name. After a power cycle, Alt+L brought the listing back, still reading the disk.
    - A snapshot made with a ROM file that was then deleted, on Quick Load, kept a byte-identical copy in `ROMs\custom.bin` and pointed `SystemROM` and `SystemROMFile` at it.
    - Save Snapshot… with a name typed without the extension saved `saved.jukusnap`.
- **Language:** Settings → Language writes `Language` in the INI file (absent for the system default, from `GetUserPreferredUILanguages`). It takes effect on the next launch, as on the Mac.
- **Settings as built** (`settings_window.rs`, `roms.rs`, as §6.7):
  - **The window** has the Mac's three panes (§6.7), as the tabs of a tab control, which is how Windows shows a settings window's pages: **General** (Mouse Speed, Sound (Key Click, Drive Sounds, Drive Volume, Fade Out the Motor When Idle for 10 s), Language, and Windows' Files row), **Display** (CRT Effects, the five CRT sliders, 3D Frame, Brightness, Restore CRT Defaults) and **Machine** (System ROM, Expansion ROM, Timer T3.2). Each pane is the Mac's form, row for row, with its notes in a smaller gray font and its spacing (20 around, 12 between groups). It is built in code from standard controls: checkboxes, trackbars, combo boxes and push buttons, in the system's message font (Segoe UI) at the window's DPI, laid out again for a new DPI. Labels are right-aligned in a first column that all panes share, each level with the middle of its control; for a slider that is its channel (`TBM_GETCHANNELRECT`), which a trackbar draws above its middle. The controls' column is as wide as its widest control, or 260 px at 100% if that is more, and the notes wrap to it, so the longer ones (Files', Expansion ROM's, and more of them in Estonian) take two or three lines instead of widening the window; Files' note is as tall as the taller of its two texts, so the form does not move when it changes.
  - **Unlike the Mac,** the window keeps one size, fitted to the largest pane, as a property sheet does, and its title stays Settings, since the tabs name the pane. It opens on the pane it showed last in this run, General at first; it is hidden, not destroyed, when closed. A click on a tab, the arrow keys on the tabs, Ctrl+Tab or Ctrl+Page Down and Ctrl+Shift+Tab or Ctrl+Page Up change the pane, wrapping around; with Ctrl the keyboard goes to the new pane's first control. The tab control is created last, so it lies below the controls, which take the clicks on its page, and Tab reaches it after them. The controls are drawn on the page's color, which the theme draws lighter than the window's (`TABP_PANE`, read once and again on a theme change), as `EnableThemeDialogTexture` gives a property sheet's pages.
  - **Applying.** Every change writes `Juku.ini` and calls `settings_changed`, the counterpart of the Mac's defaults notification: the window state applies the ROMs and the key click (`InstalledRoms::apply`, the Mac's `applySettings:`), the drive sounds (`settings::apply_drive_sounds`), the T3.2 clock (`settings::apply_t32_clock`), the picture settings and the mouse speed, and the Settings window and the menus are brought up to date from the file. A slider being dragged changes its setting in memory and the file is written when the drag ends, since the file is rewritten whole. The keys are the Mac's: `MouseSensitivity`, `KeyClick`, `DriveSounds`, `DriveSoundsVolume`, `DriveIdleMute`, `T32FastClock`, `SystemROM`, `SystemROMFile`, `ExpansionROM`, `ExpansionROMFile`, and Windows' `Language`. Slider values are kept to five decimals.
  - **Lists.** A choice is taken when an item is picked from the open list, or with the arrow keys on the closed one (each arrow press is a choice, as in Windows' own forms). Other… puts the list back to the ROM in use and opens the file dialog (`*.bin;*.rom`, "ROM Images"). The list shows the last ROM file by its name, with its path as a tooltip while it is the one chosen. Combo boxes have no separators, so the Mac's line before the file is left out.
  - **Restore CRT Defaults** is enabled while any of the six slider keys is in `Juku.ini`, and removes them. The Drive Volume slider and the Fade Out the Motor When Idle for 10 s checkbox are grayed out while Drive Sounds is off, as on the Mac. The label of a disabled control is grayed with it, in the notes' color (`is_dimmed_label`), except a checkbox's: Sound: also heads the checkboxes under it, and a checkbox grays its own title. A control that is disabled while it has the keyboard, this button, Key Click or those two, hands it to the next one first, as a dialog does; otherwise the keyboard would be lost.
  - **ROMs.** As §6.7: a ROM file is checked when it is chosen and refused with an alert if it is not 16 KB (an expansion ROM, if `expansion::check` refuses it); choosing the same file again loads it again; a ROM that cannot be loaded later is reported and its setting goes back to what is in the machine. At launch a ROM file that has gone is reported once the window is up and the setting is removed. Key Click is grayed out, with the note shown, when the ROM has no click code the patch can change (JMON 3.3).
  - **Verified:** the keyboard alone, from Alt+,: Space on CRT Effects wrote `CRTEffects=0` and `1`; five Right presses on Curvature wrote 0.02796; Restore CRT Defaults removed it and the keyboard went on to Mouse Speed; Down on System ROM wrote `jmon33` and the screen showed MONITOR 3.3, with Key Click grayed out and its note shown; Up went back to `ekta37`; Esc hid the window and the emulator window was active again. Machine → Expansion ROM → Other… with `jbasic11.bin` cold-started the machine, and `A` at the prompt gave JBASIC's `READY`; at the next launch it was plugged in again. A disk image chosen as the system ROM was refused with "“prog.juk” is not a Juku ROM image (16 KB expected)." With `Language=et` the menus and Settings were Estonian.
  - **Verified (panes):** Alt+, opened General with the keyboard on Mouse Speed; Tab and Space wrote `KeyClick=0`. Ctrl+Tab showed Display with the keyboard on CRT Effects, and again Machine on System ROM, where Down chose JMON 3.3; Ctrl+Tab wrapped to General, which showed Key Click grayed out with its note, and Ctrl+Shift+Tab went back to Machine. The window kept its size on every pane, and after Esc and Alt+, it opened on Machine again. With the mouse, a click on Display showed it and a click on CRT Effects on its page toggled the setting; with the tabs focused, Left changed the pane, and Tab went on to the page's first control.
- **File types:** a button in Settings registers `.juk` (with `.img`, `.dsk` and `.cpm` under Open With), `.jukusnap` and the thumbnail handler for the current user, in `HKCU\Software\Classes`, so it needs no administrator rights. The installer does the same (§11.7). ProgIDs allow no hyphens and at most 39 characters, so they take the bundle identifier's names in Pascal case, without `io.github`: `SinijarvJuku.JukuEmu.DiskImage`, `SinijarvJuku.JukuEmu.Snapshot`.
  - **As built** (`file_types.rs`): Settings has a row of its own at the end of General, Files: with Register File Types and a note under it, which says either what the button does or, once the types open in this copy of Juku (the disk image type's open command names this exe), that they do. The button writes:
    - the two ProgIDs, each with its name ("Juku Disk Image", "Juku Snapshot", the Mac's type names, in the language Juku runs in), `FriendlyTypeName`, `DefaultIcon` (the exe's icon) and `shell\open\command` (`"…\Juku.exe" "%1"`); the snapshot type has `Treatment` 0, so Explorer draws no frame around its thumbnails;
    - `.juk` and `.jukusnap` pointing at them, and the disk image ProgID under `OpenWithProgids` of `.juk`, `.img`, `.dsk` and `.cpm`, so the last three offer Juku without being taken over, as the Mac's Alternate rank does;
    - the thumbnail handler's class (`CLSID\{df6fb75e-81fd-457d-be7c-f8bf50d7a27f}`, `InprocServer32` naming `JukuThumbnail.dll` beside the exe, `Apartment`) and the shell's thumbnail key (`ShellEx\{e357fccd-a995-4576-b01f-234630154e96}`) on the snapshot ProgID and on `.jukusnap`, if the DLL is there;
    - `Applications\Juku.exe` with its name, icon, open command and `SupportedTypes`, for Open With;
    - and then tells Explorer with `SHChangeNotify(SHCNE_ASSOCCHANGED)`.
    - Registering again after Juku has moved points everything at the new place. A choice the user made in Windows' own "Open with" (its `UserChoice`) still wins, as Windows intends. There is no button to unregister; the uninstaller does that (§11.7).
    - **`Juku.exe --register` and `--unregister`** do the same without a window, for the installer, and exit with 0 or 1. Unregistering acts only if the types open in this copy of Juku, so uninstalling leaves a copy elsewhere that the user registered. It deletes the two ProgIDs, the thumbnail class and `Applications\Juku.exe` (the last two only if they name this copy), Juku's ProgID among each extension's `OpenWithProgids`, an extension's default value only if it is Juku's ProgID, and then each extension key it leaves empty: what other apps keep there stays.
  - **Jump list.** The recent images and snapshots Juku opens or saves go to Windows' recent documents (`SHAddToRecentDocs`), which the taskbar's jump list shows for an app registered for their types. Windows leaves out files in the temporary folder.
  - **Verified** (the registry keys did not exist before; the test removed them afterward): after Register File Types every key above was there with this build's paths, and the note changed. Explorer's thumbnail of a snapshot (`IShellItemImageFactory`, thumbnails only, 256 px) was its picture at 256 × 192, through `JukuThumbnail.dll`; `.juk` files and the exe had Juku's icon. Opening a `.juk` file as Explorer does (its default verb) started Juku with it in A:, and a `.jukusnap` file opened the same way went to the running Juku. A `.juk` file opened from outside the temporary folder was added to the recent documents.

### 11.5 Window and rendering
- **Window:** the content keeps 4:3 while it is resized (`WM_SIZING`). The View sizes are in device-independent pixels (96 per inch), as the Mac's are in points. The placement is kept in the INI file and moved onto a monitor if its own has gone. In dark mode the title bar is dark (`DWMWA_USE_IMMERSIVE_DARK_MODE`); the other windows stay light, since the Win32 controls do not follow dark mode.
  - **As built:** the window opens centered on its monitor's work area at the largest View size that fits, 3× if it can (1148 × 861 at 100%, without the bezel margin until the CRT pass is drawn). The minimum is 1× without the margin. The aspect follows the dragged edge, and for a corner the width. The theme is read from `AppsUseLightTheme` and read again on `WM_SETTINGCHANGE` "ImmersiveColorSet". The placement is kept (below). The menu bar stays light: Windows has no supported dark menu bar.
- **Light and dark mode** (`theme.rs`): the app follows Windows' setting for apps (`AppsUseLightTheme`), read at startup and again on `WM_SETTINGCHANGE` with `ImmersiveColorSet`, when the windows are redrawn in the new mode. Win32 has supported parts and unsupported ones:
  - **Settings → Appearance** (Windows only; the Mac app follows macOS): Same as Windows, Light or Dark, in General below Language, kept as `Appearance` in `[Settings]` (`light`, `dark`, absent for Windows' setting). It takes effect at once: the mode is taken again and every window of the app's thread is sent a registered message (`JukuAppearanceChanged`), which each window handles as it does the system's change.
  - **Supported:** the dark title bar (`DWMWA_USE_IMMERSIVE_DARK_MODE`) and the Mica backdrop (`DWMWA_SYSTEMBACKDROP_TYPE`, Windows 11 22H2 and later) on the main window and Settings. Mica reaches only the title bar, since the controls draw with GDI on an opaque client area, and Windows shows it only with transparency effects on and accent color off on title bars; otherwise the title bar is a plain color.
  - **Popup menus** turn dark through `uxtheme.dll` functions exported only by ordinal, as Explorer and Notepad++ use them: 135 `SetPreferredAppMode` before the first window and on every change (1 to follow the system, 2 and 3 to force dark and light for Appearance, from build 18362; before it Appearance does not reach the popup menus), 133 `AllowDarkModeForWindow` on each window, and 104 `RefreshImmersiveColorPolicyState` and 136 `FlushMenuThemes` on a change. They are used from build 17763 (1809), whose ordinal 135, `AllowDarkModeForApp`, takes the same 1; before it the menus stay light.
  - **The menu bar** is drawn by the window in dark mode, through the undocumented messages `DefWindowProc` sends to have it drawn: `WM_UAHDRAWMENU` (0x91) fills the bar, `WM_UAHDRAWMENUITEM` (0x92) draws each title, lighter under the mouse and when open, gray when the window is inactive, with the underlined letters only when Windows shows them. After `WM_NCPAINT` and `WM_NCACTIVATE` the light line Windows draws between the bar and the client area is painted over.
  - **The other windows** take the mode the same way: the title bar, the window's color and its text in the colors of the mode. Keyboard Mapping draws its text itself.
  - **The Settings window's controls** take the dark themes Explorer's own windows use: `DarkMode_Explorer` for the push buttons, check boxes, tooltip and the lists' scroll bars, `DarkMode_CFD` for the drop-down lists. The window, the page, the labels and the open lists get the dark colors through `WM_CTLCOLORSTATIC`, `WM_CTLCOLORBTN` and `WM_CTLCOLORLISTBOX`. Custom draw covers what those themes leave light: the sliders' channel, and the text of a push button that is off, which the dark theme draws nearly as bright as one that is on. No theme has a dark tab control, so it is subclassed and painted in the dark colors, as the light theme draws it. In light mode the controls keep the default themes.
  - **Colors** are Windows 11's: the window `#202020`, the page `#2B2B2B` with a `#404040` edge, white text and `#A0A0A0` secondary text.
  - **Checked** on Windows 11 build 26300 by screenshots: dark mode at startup, the menu bar and an open menu, each Settings pane with CRT Effects on and off, an open drop-down list, and a switch from light to dark and back while Settings was open (forced through a test hook, since removed). Older builds of Windows 10 and 11 were not tried; the check boxes' text is the part most likely to differ there.
  - **Checked** too, by screenshots with each Appearance chosen in turn while Settings and Keyboard Mapping were open: every window and the popup menus followed at once, Light included while Windows was dark.
  - The alerts (`TaskDialog`) are not themed: the task dialog has no dark theme an app can give it.
- **Pacing as built.** The main loop waits on a high-resolution waitable timer (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`), set for the next tick each time round, and on window messages. A tick is one refresh of the window's display, from `EnumDisplaySettingsW`, read again after a move or size, a DPI change and a display change; the machine runs for the time actually elapsed, so a rate of 59 for 59.94 Hz does not matter. Each tick does what the display link tick does on the Mac (§6.1): run the machine, then draw only while the picture is settling. While a modal loop runs, a pacer thread does the ticking: it waits on a high-resolution waitable timer of its own at the same period and, once the main loop has missed two ticks, posts `WM_APP_TICK` to the window each period, which the modal loop delivers and which does the whole tick, drawing included. It posts again only after the window has taken the last one, so a busy window does not pile them up. The picture so keeps moving at the display's rate while the window is dragged or a menu or dialog is open. (A 20 ms `SetTimer` did this at first, but `WM_TIMER` comes at the system timer's 15.6 ms granularity and the missed-two-intervals test halved that again, so the picture moved at about 16 frames a second, choppy behind an open menu.) A press on the title bar is the exception: before Windows starts the loop that moves the window, it waits up to half a second for the mouse to move, handing out no messages, so the picture stopped for that long. The wait holds up every window of the thread, so a press on the title bar of Settings, Keyboard Mapping or the import window stops the picture too. The main window and each of those post themselves a `WM_MOUSEMOVE` on `WM_NCLBUTTONDOWN` at `HTCAPTION` (`emu_window::title_bar_pressed`), which ends the wait at once; the move loop then starts, and ends on the release if the mouse did not move. Checked with synthesized input, on the main window and on Settings (where the picture had stopped for 502 ms before): no gap in the ticks while the title bar is held, a drag moves the window by exactly the mouse's distance, and a double click still maximizes. A press on the frame starts the sizing loop at once and needs nothing. A minimized window is not drawn. A covered one still is: Windows has no occlusion state like the Mac's, and a flip-model swap chain reports `DXGI_STATUS_OCCLUDED` only in some cases.
  - **No blocking.** The swap chain allows one frame queued (`SetMaximumFrameLatency(1)`), and a draw first checks its frame-latency waitable object with no timeout: when the display is not ready, the draw waits for the next tick instead of stalling the machine.
  - **Measured** (i9-13900K, 1148 × 861 at 100%, boot and monitor prompt, release build): 0.66 s of CPU over 9.6 s, about 7% of one core, and an 83 MB working set, most of it the Direct3D runtime and driver.
- **Direct3D 12:** feature level 11_0, a flip-model swap chain (`FLIP_DISCARD`) of BGRA8 buffers drawn through an sRGB render target view, its frame-latency waitable object, and three frames in flight with a fence, like the Mac's semaphore (§7.4). Direct3D 12 cannot make mipmaps itself, so a downsample pass builds those of the persistence texture for the bezel's light (§7.3a). A removed device (driver update, GPU reset) is recreated.
- **Snapshot thumbnail:** drawn off screen at 640 × 480 as in §6.8, read back, and encoded as PNG with WIC.
  - **As built:** `Renderer::thumbnail` runs decode, persistence (with no picture before it, so no afterglow), the mipmaps and the CRT or flat pass into textures of its own, made the first time and kept: a beam, a phosphor with its mip chain, a 640 × 480 `B8G8R8A8_UNORM_SRGB` target, and a readback buffer the target is copied into. Their descriptor tables and render target views follow the window's in the same heaps. It waits for the draws in flight first, so it can use their command list. The uniforms are those of a window, so the bezel shows all round. `snapshots.rs` encodes the pixels as a 24-bit PNG with WIC (`IWICBitmapEncoder` into a memory stream), 148 KB for a screen full of EKDOS's directory listing (WIC adds the sRGB and gamma chunks); if any step fails, the core's 1-bit picture is used.
- **As built:**
  - **CRT pass.** With CRT Effects on, the output pass is `ps_crt` on the phosphor, as on the Mac; off, `ps_flat` on the beam. CRT Effects is on by default, as on the Mac.
  - **Mipmaps.** With CRT Effects and 3D Frame on, each draw makes the mip chain of the phosphor it has just written (9 levels from 416 × 287): `ps_downsample` draws each level as `fsh_downsample` does on the Mac (§7.3a). The first version averaged the 2 × 2 texels at twice the position, which drops the last row or column of an odd level and puts the deeper levels' light out of place. Each level has a render target view of its own and a table whose t0 is the one level above; a barrier moves each level to shader-resource state once it has been drawn, so at the end of the chain the whole texture is ready for `ps_crt`, which samples it through all its levels. 
  - **Settings.** The picture settings are read from `Juku.ini` (`[Settings]`, the Mac's keys `CRTEffects`, `CRTCurvature`, `CRTScanlines`, `CRTGlow`, `CRTBackground`, `CRTFrame`, `CRTBezel3D`, `CRTBrightness`), with the Mac's defaults (§7.3a) for what the file does not hold. View → CRT Effects (Alt+E) writes `CRTEffects` as 1 or 0. The other keys have their sliders in the Settings window.
  - **View menu** (as §7.3a): CRT Effects, the five sizes, Enter Full Screen (Alt+Enter). The sizes are in device-independent pixels, so at 100% 1× is 419 × 314 with the bezel margin and 1148 × 861 is 3× without it; the titles use the decimal mark of the user's region (`GetLocaleInfoEx`, `LOCALE_SDECIMAL`), 1,5× in Estonian. A size is ticked when the tube is at it within one point, and disabled in full screen and when the window would not fit the monitor's work area. Choosing one restores a maximized window, keeps the top left corner and moves the window only to keep it on the work area. The full screen item reads Exit Full Screen in full screen, as AppKit makes the Mac's.
  - **Full screen.** The window's placement is saved, its style becomes `WS_POPUP` and the menu bar is taken away, and it covers its monitor (`rcMonitor`). Leaving puts the style, the menu bar and the placement back. The flip-model swap chain needs no mode change. Windows has no menu bar that appears at the top edge, so **F10 opens the menus as a popup menu** at the top left corner: a popup holding the menu bar's submenus, the same menu handles, so they stay up to date. Alt shortcuts work as in a window.
  - **The cursor in full screen** (§6.2): hidden on entering, shown when the mouse moves, and hidden again after 3 s without movement, through `WM_SETCURSOR`. It stays shown while a menu is open, while a mouse button is held, when it is outside the window and when Juku is not the active window. Windows sends `WM_MOUSEMOVE` without movement too (when a window above changes), so only a new position counts.
  - **Placement.** `[Window]` holds `Frame` (the normal frame, `left,top,right,bottom`) and `Maximized`, saved when a move or size ends, on full screen and on quit; in full screen the frame from before it. At launch a frame that is on no monitor now is ignored and the window opens as the first time.
  - **Verified** (debug build, debug layer silent throughout; 100% scale on a 1920 × 1080 monitor): with no `Juku.ini` the window opened at 3× with the bezel, 1257 × 943. View → 1× gave 419 × 314 and 2.5× 1048 × 786. Alt+E turned CRT Effects off (`CRTEffects=0`) and back. Alt+Enter covered the monitor, 1920 × 1080, with the tube at full height and the bezel at the sides; the cursor was hidden 3.5 s later; F10 opened the menus in the corner; Alt+Enter brought back the window at its old frame. A window moved to 140, 90 opened there again at the next launch.
  - **Measured** (release build, i9-13900K, 1257 × 943, boot and 8 s at the prompt, 10.4 s in all): 1.16 s of CPU with CRT Effects off, 1.39 s with them on and 3D Frame off, 1.61 s with both, so 11%, 13% and 15.5% of one core. The process's time includes the audio thread and the graphics driver's threads; the extra passes cost the driver about 4% of a core. An earlier measurement gave 7% for the flat picture, before the audio thread; what accounts for the difference has not been measured.

### 11.6 Explorer thumbnails
The counterpart of the Quick Look extension: a DLL implementing `IThumbnailProvider` and `IInitializeWithStream`, in the crate `frontends/windows/thumbnail/`. It decodes the snapshot's PNG with WIC, which skips the private chunks, and scales the picture to the size asked for. The Settings button (§11.4) registers it.
- **As built:** `JukuThumbnail.dll` is a `cdylib` with the app's dependencies and release profile, built by `build.cmd`. It exports `DllGetClassObject`, which hands out a class factory for its one class, and `DllCanUnloadNow`, which always says no: the DLL keeps no state worth the bookkeeping. Its `build.rs` exports both `PRIVATE`, out of the import library, as the linker wants for these names (LNK4104). `IInitializeWithStream::Initialize` reads the whole file (at most 16 MB), since the stream is only good during that call; `GetThumbnail` decodes it with WIC, which finds the PNG by its signature whatever the extension, scales it with WIC's high-quality cubic scaler so its longer side is the size asked for (larger too, as the Mac's does), converts it to 32-bit BGRA and returns it as a top-down DIB section, opaque (`WTSAT_RGB`). Explorer runs it in an isolated process of its own. The class ID is written in both crates, with a comment in each.

### 11.7 Installer

`frontends/windows/installer/Juku.iss` makes `Juku-<version>-windows-x64-setup.exe` with Inno Setup 6 (https://jrsoftware.org/isinfo.php, by Jordan Russell and Martijn Laan, free for any use). Chosen over MSIX, which needs a trusted signature to install at all and redirects a packaged app's registry and AppData writes (the Register File Types button and `Juku.ini` would not work as they do), and over WiX (MSI), which is more work for no gain at Juku's size.

- **For the current user only** (`PrivilegesRequired=lowest`): it installs into `%LOCALAPPDATA%\Programs\Juku` with no administrator rights, which school computers often refuse, and everything it registers is in the user's part of the registry, as Register File Types writes. 64-bit Windows 10 or 11.
- **What it does:** copies `Juku.exe` and `JukuThumbnail.dll`, adds a Start menu shortcut and, if asked, one on the desktop, and runs `Juku.exe --register` for the task "Open .juk disk images and .jukusnap snapshots in Juku…", which is ticked by default. Uninstalling runs `Juku.exe --unregister` first, then removes the files, the shortcuts and its entry in Installed apps. The settings and files in `%LOCALAPPDATA%\Juku` stay, as the Mac's do when the app is deleted. Juku and Explorer's thumbnail process are asked to close if they hold the files (`CloseApplications`).
- **One source for the version:** the script reads the version, the publisher and the copyright from `Juku.exe`'s version information (`GetStringFileInfo`), which `build.rs` takes from `VERSION` and `Info.plist`; the publisher is the copyright holder, also `CompanyName`. The `AppId` GUID identifies the installation across versions and must not change.
- **Built by `build.cmd dist`** after the zip, if Inno Setup 6 is found: `ISCC.exe` on the PATH, in the `ISCC` environment variable, or where its installer puts it for the user or for all users. Without it `dist` makes the zip alone and says so. The setup program is 2.4 MB, most of it Inno Setup's own.
- **Juku's artwork** replaces Inno Setup's: `make_artwork.ps1` (§11.1) also makes the small image at the top right of each page, the icon at 58 to 145 px, and the side panel of the last page, the icon's gradient with its lettering and LED across the panel's width, at 202 × 386 to 505 × 965 px. Both are PNG with the scales 100% to 250% that Inno Setup 6.7's modern style asks for (its small image area is square), and Setup picks the best fit.
- **English only**, by decision: Inno Setup has no official Estonian translation, and the installer's few pages do not need one. The app itself follows the language setting (§11.4).
- **Not signed.** Like the zip, the setup program gets Windows SmartScreen's warning until it has a reputation or a signature.
- **Verified:** a silent install (`/VERYSILENT /TASKS=filetypes`) put the files in `%LOCALAPPDATA%\Programs\Juku`, the Start menu shortcut, the entry in Installed apps (Juku, 0.1, the publisher, Juku's icon) and the nine file type keys pointing at the installed copy, with no prompt; the installed Juku started. The silent uninstall removed the files, the shortcut, the entry and every key, and kept `Juku.ini`. A value another app kept under `.img\OpenWithProgids` was still there after the uninstall. The wizard shows the task ticked and the desktop shortcut not.

### 11.8 Debug server

The guest debugger (§12), served as the Mac app serves it (§12.3, §12.7).

- **The crate.** `juku-windows` depends on `juku-dap` by path and calls its `Server` directly, as it calls the core (§11.1); `juku-dap`'s C ABI is the Mac's only.
- **Ticks** (`emu_window.rs`, as `EmuView.m`): each tick polls the server before it runs the machine, and while the server is on the machine runs through `Server::run`, in warp's slices too, then `Server::poll`. The window keeps the last state (connected, stopped). A stop by the debugger is the app's Pause: Machine → Pause is ticked, Alt+P goes on (`Server::set_paused`), and a client that connects to a paused machine finds it stopped. The title shows Debugging before the paused or warp state, joined as on the Mac: `Juku — Debugging · Paused`.
- **Settings.** The Machine pane's last row is the Mac's Debugger: Accept Connections (`AcceptDebugger` in `[Settings]`), with the note "VS Code and other DAP clients connect on port 5104, from this PC only.", a Windows-only string, since the Mac's names the Mac. The port is `DebuggerPort` in `[Settings]` (5104 when absent), with no control of its own: it is changed in `Juku.ini` while Juku is not running. A port that cannot be had is reported once, in a task dialog with the error icon, the Mac's message and its note ("Another program may be using it.").
- **Command line.** `--debug-server` turns the server on for the run, on `DebuggerPort`, and `--debug-server=<port>` on that port; it wins over the setting. Any other argument is the file to open (§11.4).
- **A Juku already running takes the port.** A second start with `--debug-server` hands the port to the running Juku with `WM_COPYDATA` (kind 2, the port as a `u16`), as it hands over a file (§11.4), and exits. The running Juku listens on that port from then on, whatever the setting says, until it quits. On the Mac `open -a` passes no arguments to an app that runs, so Accept Connections must be ticked there; on Windows VS Code's extension can start Juku whether or not one is running (§12.8).
- **A launch's snapshot** (§12.7). After each run and poll the window asks the server for a snapshot to open (`Server::take_request`) and posts its path to itself (`WM_APP_DEBUG_SNAPSHOT`). After the tick, `snapshots::open_for_debugger` opens it as a snapshot from Explorer is opened, the changed disks asked about first, but leaves the jump list alone; the server hears whether it opened at once, or at the next tick if the window's state is busy (`Server::snapshot_opened`). A snapshot asked for while the save alert is open is reported as not opened.
- **Size.** `Juku.exe` went from 642 560 to 771 072 bytes (release build, this PC's CPU), 126 KB more. By the linker map, the server's own functions are about 85 KB of it; the rest is std's sockets and the core's debugging loop (§12.4). That is more than the tens of kilobytes §12.1 aims at, and has not been looked into yet. The server's `opt-level = "s"` (§3.1) applies to its crate before link-time optimization, which runs at the app's level.
- **Checked** (release build, Windows 11 build 26300, with a DAP client in Python on the socket; `Juku.ini` and Quick Save copied before and put back after):
  - Started with `--debug-server=5199`: after `initialize` and `attach`, `pause` brought `stopped` 10 ms after the connection, at D9CCh. Twenty instruction steps (`next` and its `stopped`) took 4.2 ms each on average and 4.6 ms at most. The title read `Juku — Debugging · Paused` while stopped, `Juku — Debugging` once continued, and `Juku` after `disconnect`.
  - A launch of a 7-byte program with `reset` and `stopOnEntry` stopped at its entry, 0100h, 2.2 s after `configurationDone`; it stepped, restarted to the entry, and ended at MONITOR with `exited` (0) and `terminated`. A launch from a snapshot taken at the prompt stopped at the entry 6 ms after `configurationDone`, and again after a restart.
  - With Juku running and Accept Connections off, a second start with `--debug-server=5198` exited, and the running Juku took a client on 5198.
  - Accept Connections in Settings started and stopped the server on 5104. With 5104 held by another program, ticking it showed the alert.

---

## 12. Guest debugger

Status: built in the Mac app: the core's part, the server, debugging from source with the assembler's debug information, launching a program into memory, watchpoints, expressions, conditional breakpoints, log points, the device and symbol views and the system's symbols, and launching from a snapshot with restart; the Windows app has it too (§11.8). This section is the plan, with "as built" notes, as in §11.

The emulator is a debug server. A client connects to it and stops, steps and inspects the emulated machine: the CPU, memory, ports and devices. The client is VS Code, through a small extension, which makes it a development environment for Juku programs: breakpoints and stepping in `.asm` files assembled with debug information. Anything else on the machine (the ROMs, EKDOS, programs from disks) is debugged by instruction, in VS Code's Disassembly view.

### 12.1 Scope
- **Protocol: DAP**, the Debug Adapter Protocol that VS Code and other editors speak. The emulator is the debug adapter itself; there is no adapter process in between. Neovim's nvim-dap can connect to the same server with no extension.
- **What it does:**
  - pause at any moment, wherever the CPU is
  - step by instruction or by source line, over a call and out of one
  - breakpoints on source lines and on addresses, with conditions, hit counts and log messages
  - watchpoints on memory reads and writes, and on I/O ports
  - registers, flags, memory, the call stack and the devices' state, and changing them
  - loading a program into memory and starting it, with its symbols and line numbers
- **Mac first**; the Windows app follows.
  - *As built:* the Windows app has the server too (§11.8).
- **Not planned:** more than one client at a time, reverse stepping (snapshots could give it later), a debugger window in the app.
- **Postponed:** a full-screen terminal client (§12.9), until VS Code has been used for a while.
- **Three rules** for every part of it:
  - **No cost without a client.** With the server off, or on with no client connected, the emulator runs the code it runs today: nothing is added per instruction or per memory access (§12.4).
  - **Few dependencies.** Each crate or package is weighed for what it brings, its quality and its size. Where one is a large answer to a simple need (JSON, base64), the parts needed are written here instead. The server has no dependencies but the core and std.
  - **A small app.** The Mac app's executable was 246 KB before the debugger. The debugger should add tens of kilobytes, not hundreds; and the executable's size is measured as it grows (§12.4).

### 12.2 Pieces

```
VS Code + extension ──DAP, TCP 127.0.0.1──► emulation thread, every tick:
  (juku-sdk's vscode/)                       Server::poll (juku-dap, non-blocking sockets)
                                               └► Session (juku-dap) ──► Machine (core: debug.rs)
```

| Piece | Where | What |
|---|---|---|
| Debug hooks | `core/src/debug.rs` | Breakpoints, watchpoints, steps, stop reasons and the shadow call stack (§12.4), in the run loop. No dependencies and no OS APIs, like the rest of the core |
| Disassembler | `core/src/cpu/disasm.rs` | Intel mnemonics, one instruction at a time with its length; checked against juku-sdk's instruction table |
| DAP server | `debugger/dap/`, crate `juku-dap` | The messages, the session (§12.5), debug information (§12.6), the TCP listener. Depends on the core only (below) |
| VS Code extension | `../juku-sdk/vscode/`, extension `juku` | The debug type `juku` and its settings, the `.asm` language; connects to the server and starts the app (§12.8). JavaScript |
| Debug information | `../juku-sdk/crates/asm`, crate `juku-asm` | The assembler writes a `.dbg` file beside the image: load address, entry, symbols, line table (§12.6) |

- **Linking on the Mac.** The app links one Rust static library, since two would each bring their own std. `juku-dap` is that library (`crate-type = ["staticlib", "rlib"]`), with the core in it as an rlib, and `build.sh` links `libjuku_dap.a` in place of `libjuku.a`. The core's C ABI (§5) comes along: a static library exports the `#[no_mangle]` functions of every crate in it (checked: the app links with it).
- **On Windows** the app is Rust: `juku-windows` depends on `juku-dap` as a crate and calls `Server` (§11.8). The static library and its C ABI are the Mac's.
- **No external crates in the server.**
  - *JSON:* `serde_json` with `serde`'s derive is good code, but brings `syn`, `quote` and `proc-macro2` to the build and a generic (de)serializer for each message type to the binary. DAP needs a value tree, a parser and a writer: about 400 lines with the string escapes and UTF-16 surrogate pairs, in `juku-dap`'s `json.rs`, tested against the cases of RFC 8259.
  - *Base64* for `readMemory` and `writeMemory`: about 40 lines.
  - *Sockets:* std's `TcpListener` and `TcpStream`, non-blocking. No threads, channels or locks (§12.3).
  - *No `format!`.* The app had none of the core library's formatting code before the debugger; numbers are written as text by hand (`text.rs`, and `Instr::text` in the disassembler), and JSON keeps numbers as integers, so no floating-point parsing or printing comes in either.
- **No workspace,** as for the Windows crate (§11.1): `debugger/dap/Cargo.toml` names the core by path and has its own release profile and `Cargo.lock`.
  - *As built:* one workspace for all the crates (§3.1). The server's profile override (`opt-level = "s"`) is in the root `Cargo.toml`, and in dev builds the server is built at `opt-level = 0`, the core at 1.
- **FFI** for the Mac (`debugger/dap/src/ffi.rs`): `juku_dap_new(port)` starts listening and returns a handle, or NULL if the port is taken; `juku_dap_poll(d, m)` does the server's work and returns the state bits (connected, stopped); `juku_dap_set_paused(d, m, paused)` is the app's Pause while a client is connected; `juku_dap_free(d, m)` closes it, and the machine runs on.
  - *As built:* `juku_dap_run(d, m, cycles, &ran)` runs the machine in place of `juku_run_cycles` while the server is on (`Server::run`). A stop the session goes on from, at a log point or a breakpoint whose condition does not hold, ends `run_cycles` early; `run` handles it and runs the rest of the cycles, so that such a breakpoint costs the machine no time. In the app, a log point on CONSTAT at the monitor's prompt printed 3 554 lines a second with the machine at 0.99× real time. A client that does not read its output is let go once 16 MB of it waits.

### 12.3 Connection
- **On 127.0.0.1 only,** port 5104 by default. Nothing outside the computer can reach it, which matters since a client can write any byte of memory; and the macOS firewall does not ask.
- **Off by default.** Settings → Machine gets an Accept Debugger Connections checkbox with the port. `--debug-server[=port]` on the command line turns it on for that run; that is how the extension starts the app (§12.8).
  - *As built:* the Machine pane's last row is Debugger: Accept Connections (`AcceptDebugger`), with a note naming the port: "VS Code and other DAP clients connect on port 5104, from this Mac only." The port (`DebuggerPort`, 5104) has no control of its own: `defaults write io.github.sinijarv-juku.juku-emu DebuggerPort <n>` changes it, and `--debug-server=<n>` uses another for one run. A port that cannot be had is reported once, in an alert ("The debugger cannot listen on port %d."). The Windows app has the same row, its port in `Juku.ini` (§11.8).
- **One client at a time.** A second connection is closed at once, and the extension says the emulator is already being debugged.
- **The subtitle** shows Debugging while a client is connected, before Paused and Warp Speed (§6.2): "Debugging · Paused".
- **Messages** are DAP's: a `Content-Length: n` header and a JSON body.
- **No threads.** The listener and the client's socket are non-blocking, and `Server::poll` does all the work on the emulation thread: it reads what has arrived, handles each whole message with the machine, reports a new stop, writes what the socket takes (keeping the rest for the next call), and accepts a client. The app calls it on every tick, before and after running the machine, so a request waits at most one tick.
  - The first plan had a thread to accept, one to read and one to write, with channels to the emulation thread and a callback to wake it. Those brought 31 KB of std into the app (threads, their spawn hooks and stack guards, channels, locks) for a latency the ticks already give.
  - *Measured* in the app, at 120 Hz: `pause` answered in 8 ms, and 20 instruction steps (`next`, then the `stopped` event) took 8.4 ms each on average and 14 ms at most.
- **Pause is shared.** A stop by the debugger is the app's Pause: Machine → Pause is ticked, and unpausing in the app continues (the client gets `continued`). Pausing in the app while a client is connected stops the machine as a `pause` request would, and the client gets `stopped`. A reset, power cycle, ROM change or snapshot opened in the app clears the shadow stack; breakpoints stay.

### 12.4 In the core
- **Two loops.** `Machine` holds `debug: Option<Box<Debug>>`, `None` until a client connects and again after it leaves. `run_cycles` checks it once per call and, while it is `None`, runs today's loop unchanged. Otherwise it runs a debugging loop of its own, in `debug.rs`.
- **A second bus.** The debugging loop does not call `Machine::step`: it steps the CPU with `DebugBus`, a wrapper around `System` that implements `Bus` by passing each call on, and looks at the calls as they go by.
  - `Cpu::step` is generic over the bus, so the compiler makes a second copy of it for `DebugBus`. The copy for `System` stays as it is, without a branch for the debugger; the price is the second copy's size, a few kilobytes.
  - `begin` gives the wrapper each instruction's opcode, `read`, `write`, `input` and `output` the accesses for watchpoints, and `int_ack` the interrupts for the shadow stack. Nothing in the CPU or in `System` changes for it.
- **What runs when:**

| State | Per instruction | Per memory access | Per tick |
|---|---|---|---|
| Server off | as today | as today | nothing |
| Server on, no client | as today | as today | one non-blocking `accept` |
| Client connected, running | the debugging loop: the breakpoint bit for PC, the step condition, the shadow stack | `DebugBus`: a watchpoint bit when any watchpoint is set | the messages that came in |
| Stopped | nothing runs | | the messages |

- **Measured.** `bench` runs its workloads before and after each change to the core's debugging, with no client, and they must stay within its noise. It gains a workload with a client's state set up (breakpoints, a watchpoint, the shadow stack), to keep the debugging loop's cost known; it is expected to run a little slower, still far above real time.
- **As built:**
  - `Machine::set_debugging(true)` creates the state, `false` drops it; `debugger()` and `debugger_mut()` reach it, and `Machine::resume` goes on from a stop. `Machine::step` stays the plain step, for tests and tools.
  - `run_cycles` tests `debug.is_some()` once and calls `run_debugging`, marked `#[inline(never)]`, so the ordinary loop is compiled as before. In the release profile the bench gave the same figures as before within 1% (§4.1).
  - LTO cannot tell that `debug` stays `None` without a server, so the debugging loop and its copy of the CPU went into the app with the debugger: 263 KB against 246 KB before.
  - Running under the debugger with a breakpoint and a watchpoint of each kind set, none of them reached: the idle prompt at 141× (172× without), `scroller.bin` at 135× (175×), in the release profile. The bench's two workloads with "debugging" in their names measure it.
  - The stepping logic is `Debug::step`, generic over the bus, so its unit tests run it on a small test bus without the machine.
  - A watchpoint hit records the access, the address and the value read or written. The old value of a write is not kept: in modes 0–2 reading it back would read the ROM or the window over that address, not the RAM written.
- **Stopping.** After each instruction the debugging loop checks a pause asked for, a breakpoint at PC (a map of 64 K bits, 8 KB), a watchpoint hit during the instruction, and the step in progress. When one holds, `run_cycles` returns early and drops the rest of its cycle target, and the machine is stopped: later calls run nothing until it is resumed. Time stops for every device, as in Pause, and the audio ring runs dry.
- **Breakpoints are on PC,** whatever the memory mode: one at E500h stops in the ROM's FDC driver in mode 1 (MD §3.3) and in RAM in mode 3. Conditions, hit counts and log messages belong to the session (§12.5): the core stops, and the session decides whether to go on.
- **Watchpoints.** A read map and a write map of 64 K bits each, and a map of 256 bits each for IN and OUT, checked in `DebugBus`. Reads of the instruction's own bytes (from PC to its length) are fetches, not data reads, and are not checked. A hit records the address and the value, and stops at the end of the instruction.
- **Steps:**
  - *Instruction:* one `Machine::step`.
  - *Over:* a CALL, a conditional call taken, or an RST runs until PC is at the next instruction with SP back where it was; anything else is a single step. A breakpoint inside the call stops it.
  - *Range:* steps while PC is in a set of address ranges, stepping over calls. The session uses it for a source line (§12.5).
  - *Out:* runs until the top frame of the shadow stack is left.
  - *Interrupts.* An interrupt taken during a step is stepped over by default: the step runs on until the handler returns, then carries on. The launch setting `stepIntoInterrupts` stops at the handler's first instruction instead.
- **Shadow call stack.** Kept by the debugging loop only, so ordinary emulation never touches it. The loop sees each instruction's opcode (`begin`) and SP before and after it, and `int_ack` marks a step that took an interrupt.
  - A CALL, RST or interrupt pushes a frame: the return address, SP after the push, the target, and whether it was an interrupt.
  - A frame is popped when the word holding its return address is taken off the stack: by a RET, or a POP or INX SP past it (a return address popped into HL for inline parameters). Loading SP (`LXI SP`, `SPHL`) switches stacks and pops nothing.
  - *As built:* at first every frame whose SP was below the present SP was popped after each instruction. RomBios's TTO moves to a stack of its own above the caller's (SP D75Bh), which popped every frame of the program calling it, so a step out of TTO stopped at once, in the ROM.
  - At most 64 frames. It is a guess about code that plays with its stack, and is shown as one: the CPU's own stack is in the memory view.
  - It knows only the calls made since the client connected. What came before shows as one frame, "…".
- **Snapshots** leave out the debug state.
- **Going on from a stop.** A stop at a breakpoint or watchpoint keeps the step it broke into; `Debug::go_on` takes it up again, for a breakpoint the session decides not to stop at. A step over a call that reaches a log point inside the call still ends after the call.
- **Known instruction starts** for the disassembly (§12.5): a 64 K-bit map of each address PC has held while debugging, cleared by a write the CPU reads back from there, and by a reset or a snapshot opened.

### 12.5 The session
It runs on the emulation thread, with the `Machine`, between `run_cycles` calls.
- **Capabilities:** `supportsConfigurationDoneRequest`, `supportsDisassembleRequest`, `supportsSteppingGranularity`, `supportsInstructionBreakpoints`, `supportsReadMemoryRequest`, `supportsWriteMemoryRequest`, `supportsDataBreakpoints`, `supportsSetVariable`, `supportsEvaluateForHovers`, `supportsConditionalBreakpoints`, `supportsHitConditionalBreakpoints`, `supportsLogPoints`, `supportsGotoTargetsRequest` (Jump to Cursor sets PC), `supportsRestartRequest`, `supportsTerminateRequest`.
- **One thread,** "8080". `stopped` gives the reason: pause, breakpoint, instruction breakpoint, data breakpoint, step or entry.
- **Stack trace** from the shadow stack. A frame is named by the symbol at or before its address (`DRAW.LOOP+3`), or by the address. Its source and line come from the line table. `instructionPointerReference` is the address, so the Disassembly view follows it, and a frame with no source opens there. Interrupt frames are labeled as such.
- **Scopes:**
  - *Registers:* A, F, B, C, D, E, H, L, BC, DE, HL, SP, PC, INTE and HALT. F opens to S, Z, AC, P and CY. All can be set. BC, DE, HL, SP and PC carry a memory reference, so the memory view opens at them.
  - *Machine:* the memory mode, the cycle count and the frame number.
  - *Devices,* read only: the 8259 (IRR, ISR, IMR), the PITs' counts, modes and outputs, the PPIs' ports, the FDC's registers and command, the drives, the video geometry and the beam's position.
  - *Symbols:* the program's labels and EQUs, with the byte and the word at each.
- **Memory.** `readMemory` and `writeMemory` take one of three references: `cpu` (what the CPU sees in the present mode), `ram` and `rom`. A write through `cpu` goes where the CPU's would.
- **Disassembly** comes from the core's disassembler, with symbols as labels and each instruction's source line where the line table has it. Going backwards from an address is ambiguous on the 8080: the session starts from the nearest line-table entry or symbol before it, or failing that tries the starts up to 3 bytes back that land on the address.
- **Source lines.** A line breakpoint is set at its line's first address. Stepping by line uses the range step over the line's bytes; `stepIn` on a line with a CALL goes into it. VS Code asks for instruction granularity while the Disassembly view has the focus.
- **Expressions** (`evaluate`, watches, hovers, breakpoint conditions): numbers as in the assembler (`0FFh`, `1010b`), registers, symbols, `+ - * / & | ^ ~ ( )`, comparisons, `[x]` for the byte at x and `w[x]` for the word. Hovering a label in the source shows its value and the byte and word there.
- **Data breakpoints** on a symbol or an address (from the Symbols scope or the memory view), for reads, writes or both; on a port, from the Devices scope's Ports.

- **As built:**
  - **Expressions** (`expr.rs`): numbers as the assembler writes them and with `0x`, characters (`'J'`), the registers in any case (A–L, F, M for the byte at HL, BC, DE, HL, SP, PC, PSW), symbols by name (in another case too, §12.6), `[x]` and `w[x]`, `HIGH()` and `LOW()`, and the assembler's operators at its precedence, with `&&` and `||` below them. `evaluate` shows a value in hex and decimal (`014Ah (330)`) with a memory reference; hovering a symbol adds the byte and word there (`014Ah (330): 7Eh, word B77Eh`); a mnemonic or anything else that is not an expression shows nothing. Registers are set with expressions too.
  - **Conditions, hit counts and log points** on line and instruction breakpoints, and conditions and hit counts on watchpoints. A hit count is a number the count must reach, or one after `>`, `>=`, `==`, `<`, `<=` or `%`. A log message's `{expression}` is written in hex, or in decimal with `,d` (`{A,d}`), and goes to the Debug Console. A condition that cannot be worked out stops, with the reason in the console. A pause that comes while the machine is stopped at a breakpoint not looked at yet becomes the stop, rather than being lost when the session goes on (`busy_log_point`).
  - **Watchpoints** (`dataBreakpointInfo`, `setDataBreakpoints`): on a symbol, from the Symbols scope, over the bytes its line of data makes (a `DW`: two; a `DS 3` after `BSS`: three), its size in the debug information, or one; on an address from an expression, with `bytes` (`supportsDataBreakpointBytes`); on a port, from Devices → Ports, where read is IN and write is OUT. Ids are `mem:7D98:0001` and `port:15`.
  - **Scopes:** Registers; Devices, with Interrupts (IRR, ISR, IMR), Timer 1–3 (each counter's mode, count, value, OUT and gate), PPI 1 and 2 (ports A–C), Floppy (registers, lines, drives), Video (the picture's size and place, the beam's line and µs) and Ports; Symbols, the labels by address with the byte and word at each, marked expensive so that VS Code opens it only when asked; and Machine. Reading the devices brings the timers up to date first (`Machine::devices`), which changes nothing they do.
  - **Jump to Cursor** (`gotoTargets`, `goto`) on a line of source: PC goes to its code, and the stop is reported as `goto`.
  - **Frames** are named by the label at their routine's entry first, so that RomBios's routines have their names (`TTO`).
- **As built:**
  - Without debug information every step is by instruction: `stepIn` is one instruction, `next` steps over a call, `stepOut` runs out of the innermost frame, whatever the granularity asked for. `setBreakpoints` answers each source breakpoint as not verified ("No debug information for this file"); `launch` is refused ("use attach"). `evaluate` is not supported yet.
  - Frames are named by the address their routine was entered at (`FF98h`), with "(interrupt)" for a handler; the outermost known one is "? at 0107h", and "…" below it is a label frame. Registers show in hex with an `h` (`3Eh`, `D6DCh`); a new value can be typed as `3Eh`, `0x3E`, `101b` or in decimal.
  - The Machine scope shows the memory mode, the cycle count and the frame number. The Devices and Symbols scopes are §12.5's.
  - Memory references are `0x1234` (what the CPU sees), `ram:0x1234` and `rom:0x1234`; the ROM cannot be written. Disassembly is of what the CPU sees now: code at 062Dh in mode 0 reads as NOPs once the machine is back in mode 1.
  - Backwards from an address the session decodes forwards from 3 bytes an instruction back and 16 more, and takes the first start that lands on the address. RomBios has places where no start does, such as D9CCh, which the prompt's loop calls: every run of decoding meets a 3-byte `JPE` at D9CAh. Then the longest run that lands is shown, and the bytes before it as data (`DB EA`).
  - **Known starts.** The core notes each address PC holds while debugging (`Debug::code_start`), and forgets one that is written to where the write is read back (`Bus::reads_back`: not a write to the RAM under the ROM or the window). An address set into PC from the client is noted too. Decoding, forwards and backwards, never runs over a known start: an instruction that would is shown as a byte of data. Without this, VS Code lost the current instruction: it keeps every line it is sent for the whole session, merging new batches over old ones by address, and marks the current instruction only on a line that starts at its address. A batch decoded from further back showed D9CAh as `JPE` over D9CCh, and a stop at D9CCh then marked nothing (found in VS Code).
  - **Stale lines.** VS Code asks for lines again only for an instruction reference it has not seen; `invalidated` and `memory` events do not reload its Disassembly view. The session notes what it has sent: when a frame's address was last sent inside a line, as when the CPU stops in code shown before it ran, the frame's reference gets one more leading zero (`0x08001`), and VS Code fetches the lines around it anew.
  - Places outside the address space are sent with the address `-1`, which VS Code leaves out.
  - A breakpoint's `stopped` event names its id, the address plus 1.

### 12.6 Debug information
- **`juku-asm bounce.asm bounce.bin -g bounce.dbg`** writes the debug information, in JSON. Version 2:
  ```json
  {
    "version": 2,
    "image": "bounce.bin",
    "load": 256,
    "size": 458,
    "entry": 256,
    "files": ["bounce.asm"],
    "lines": [
      [256,0,30,3],
      [263,0,34,2,1,3,1,3,1,3]
    ],
    "data": [[259,4],[340,374]],
    "labels": [
      ["start",263],
      ["setup",340,4]
    ],
    "equs": [
      ["TTO",65497]
    ]
  }
  ```
  - Paths are relative to the `.dbg` file.
  - `lines` has a row for each run of code from one file with no gap between its lines: the address, the file, the first line and that line's length in bytes, then for each line after it how many lines on it is (0 in a `REPT` body repeated on one line) and its length. A run ends where the code skips bytes or the file changes, or a line goes back, as each time round a `REPT` body does.
  - `data` is the ranges DB, DW, DS, ALIGN and INCBIN made, joined where they meet.
  - `labels` are `[name, address]`, with the size of the line of data or the memory reserved after `BSS` at the label as a third field: what a watchpoint on it watches. `equs` are `[name, value]`. Both are in order of value.
  - Memory reserved past the image (a `BSS` area) is named in the labels, not in the lines.
  - *Version 1* had a row `[address, length, file, line]` for each line, of code and of data, and each symbol with its kind, one to a row with spaces. Version 2 holds the same in about a fifth of the space: bounce's file went from 1.9 KB to 0.7 KB and scroller's from 10.5 KB to 3.3 KB. The loader takes version 2 only; `resources/symbols/` is in it too.
  - *As built:* `-g` takes the file's name, as `-l` and `-s` do. The demos' `.dbg` files are build output, left out of the repository (`.gitignore`). The server's tests assemble `bounce.asm` with `juku-asm`'s library and write `bounce.bin` and `bounce.dbg` into their `CARGO_TARGET_TMPDIR`.
  - *As built:* the server's dev-dependencies, `juku-asm` and `juku-i8080`, are git dependencies on `https://github.com/sinijarv-juku/juku-sdk` at a release tag, so that a clone of this repository alone builds and tests. Cargo reads every dependency of the workspace, dev-dependencies too, before it builds anything, so with the path dependency even `build.sh` needed juku-sdk beside it. To work on both, a `[patch]` of that URL in a cargo configuration outside both repositories (`../.cargo/config.toml`, which cargo finds in a parent folder) puts in the copy beside this one; docs/Development.md says how. While it is in place, `Cargo.lock` names the local copy, without a source.
  - *As built:* names are case-sensitive, as written: `count:` and `COUNT equ 3` are two symbols; mnemonics, registers, directives and `high`/`low` are not. The server finds a name typed in another case when only one matches it ignoring case (`Info::value`): `PRINT` is `print` and `conch` RomBios's `CONCH`; an exact match comes first, and two that match ignoring case (`Count` for `count` and `COUNT`) are an error naming both. Registers are taken in any case before symbols.
- **System symbols.** `resources/symbols/rombios.dbg` names the RomBios entry points and system variables (MD §14.2, §14.3), and `ekdos.dbg` the EKDOS addresses (MD §3.4). They are written from the machine description, not taken from anyone's software. The RomBios file is loaded when the ROM's jump table at FF26h is RomBios's, the EKDOS one when EKDOS is found in memory (§12.7); the launch setting `systemSymbols: false` leaves them out.
  - *As built:* the files are built into the server (`include_str!`) and loaded at attach and launch. RomBios's when CONSTAT, MONITOR, TTI and TTO (FF98h, FFC4h, FFD3h, FFD9h) hold JMPs or CALLs and the pointer at FFA5h points to D4BEh, the keyboard variables: both RomBios 3.43 ROMs, every entry of `rombios.dbg` checked against them; not JMON 3.3. EKDOS's (WBOOT, BDOS, TFCB, TFCB2, TBUFF) when 0000h and 0005h hold JMPs. All are labels, so that a call shows as `CALL CONSTAT` and a load as `LDA CONCH`.
  - *As built:* the names and values are juku-sdk's include files, `include/rombios.inc` and `ekdos.inc`, which `juku-asm` has built in and programs include (the demos do). The `.dbg` files here hold the same names as labels; the server's test `system_symbols_are_the_sdks` assembles each include file with `juku-asm` and compares. `ports.inc` names the I/O ports (`PIC_0`, `PPI1_A`, `T2_1`, `FDC_CMD`, …), which the server's Ports view names by chip instead. The server's tests find `bounce.asm`'s lines by their text, so that the demo can change above them.

### 12.7 Launch and attach
- **Attach** connects to the machine as it runs. Nothing is loaded or changed; it stops when asked or at a breakpoint. `debugInfo` lists `.dbg` files to load, for a program that is already in memory.
  - *As built:* `debugInfo` is a path or a list of them. A file that cannot be read or is not debug information fails the attach, with the reason. The session sends `initialized` after the attach, so that the breakpoints VS Code sends then find the debug information loaded.
- **Launch** loads a program into memory and starts it:
  ```json
  {
    "type": "juku",
    "request": "launch",
    "name": "Bounce",
    "program": "${workspaceFolder}/demos/bounce.bin",
    "stopOnEntry": true
  }
  ```
  1. With `snapshot`, the session first opens that snapshot (one taken at EKDOS's `A>` prompt, say), so every run starts from the same state; with `"reset": true` it power-cycles instead. Otherwise the machine stays as it is.
  2. It waits for a **safe point**: the guest waiting for a key in RomBios's keyboard input, as at the monitor's `*` prompt and at EKDOS's `A>`. Taking over there leaves no interrupt handler, disk command or BIOS call half done. Which address that is in each bundled ROM, and that EKDOS's console input goes through it, are checked (below). If none comes within 10 s, the launch fails with "The machine is not at a prompt".
  3. It writes the image into RAM at `load`, from the `.dbg` file or the launch setting, 0100h if neither gives one.
  4. It pushes the **exit address** and sets PC to `entry`. The exit address is 0000h under EKDOS (a warm boot; EKDOS is known by the JMPs at 0000h and 0005h) and FFC4h otherwise (MONITOR), unless the `exit` setting gives one. Under EKDOS the command tail at 0080h is set from `args`, and the FCBs at 005Ch and 006Ch are filled from it, as the CCP does.
  5. It sets the breakpoints VS Code has sent, and stops at the entry with `stopOnEntry` or runs.
- **The end.** When PC reaches the exit address, the session sends `exited` and `terminated`, and the machine carries on. **Restart** launches again; from a snapshot, each run starts the same way. **Disconnect** clears the breakpoints and watchpoints and lets the machine run.
- **Not in the first version:** running a COM file from a disk through EKDOS, and loading a program as an expansion ROM.
- **As built:**
  - **The safe point.** The key wait in RomBios's TTI (`JMP D9D9h` at FFD3h) calls CONSTAT (FF98h) over and over, from D9D0h in RomBios 3.43m and D9CFh in 3.43 #0031, at the monitor's prompt and at EKDOS's alike: EKDOS's console input goes through it. The session sets a breakpoint of its own at FF98h, and takes a stop there as the safe point when the return address on the stack is in the ROM (D800h and up, in mode 1 or 2); a program polling CONSTAT itself calls it from RAM and runs on. JMON 3.3 has another layout, and a launch there fails after the wait.
  - **EKDOS** is taken to be there when 0000h and 0005h hold JMPs and SP is below D400h (EKDOS's BIOS waits on a stack at D2F4h; the monitor's is at D450h). The program's stack then starts at BDOS's address, from the JMP at 0005h, with 0000h pushed, as CP/M programs expect; a program that tells EKDOS from the monitor by its stack sees EKDOS. Under the monitor the program runs on the monitor's stack, with FFC4h pushed. A program that would reach BDOS, or the monitor's workspace at D300h, is refused.
  - **The command line** under EKDOS: `args` in capitals at 0080h after its length and a blank, ended by a 00h, and the first two words as FCBs at 005Ch and 006Ch: drive (0 by default), name and type blank padded, `*` filling the rest of its part with `?`.
  - **Order.** The launch request reads the program and its debug information (`debugInfo`, or the `.dbg` file beside the program); `configurationDone`, after the breakpoints, starts the wait, with the machine running. The safe point's stop and the program's end are the session's own: neither is reported as a stop. With `stopOnEntry` the machine stays stopped before the first instruction, reported as `entry`. The shadow stack and the known code starts are forgotten when the program goes in, and the stack trace has no "…" frame: the outermost frame is the program's, named by its entry.
  - **The end.** A breakpoint of the session's own at the exit address; reaching it sends `exited` (code 0) and `terminated`, and the machine runs on: EKDOS's warm boot brings back `A>`.
  - **No safe point** within ten seconds of the machine's time: an `output` event in the `important` category, which VS Code shows as a notice ("The machine is not at a prompt: …"), and `terminated`. Nothing is written.
  - `reset: true` power cycles the machine at `configurationDone`, so the program starts at the monitor's prompt 2 s later.
- **As built:**
  - **A snapshot** (`snapshot`) is opened by the host, as the user would open it, so that the app's own state follows: the ROM and expansion ROM settings, the disks and their names, the question about a changed disk. At `configurationDone` the session asks for it (`Server::take_request`; on the Mac `juku_dap_snapshot_request`; on Windows §11.8), the app opens it after the tick (`openSnapshotForDebugger:`, the path Finder's Open takes) and answers (`juku_dap_snapshot_opened`); then the wait for a safe point begins. A snapshot that does not open, or whose opening the user cancels, ends the launch with "The snapshot was not opened".
  - **Restart** (`supportsRestartRequest`) launches again with the settings VS Code sends, or those of the launch: the program and its debug information are read again, line breakpoints found again in it, and the machine brought back to a prompt by opening the snapshot again, or power cycling for `reset`. Without either the program runs where no prompt comes, so the restart is refused and says why. The session and its breakpoints and watchpoints stay.
  - **Checked:** in tests, a program launched from a snapshot taken at the prompt, into a machine that had run on and had a key typed, stopped at its entry; two restarts, each after a step into the program, stopped at the entry with the machine the same to the byte (its snapshot) and at the same cycle. In the app, a launch from a snapshot stopped at its entry 17 ms after it was asked, at cycle 6 000 368, and twice again after restarts. (The app's settings, which opening a snapshot changes, were saved before and put back after: `defaults export` and `import`.)
  - **Checked:** a program assembled with `-g` and launched under EKDOS 2.30 (booted from `jukprog1.juk`) stopped at its entry, at its first line of source, ran, and left through `JMP 0`, which sent `exited` and `terminated`; `A>` came back 1.6 s later. In the app, started as the extension starts it (`open -a … --args --debug-server=5199`), a program launched at the monitor's prompt stopped at its entry 0.1 s after `configurationDone` and ended at MONITOR.

### 12.8 VS Code extension
- *As built:* the extension moved from `debugger/vscode` to juku-sdk's `vscode/`, renamed from `juku-debug` ("Juku Debugger") to `juku` ("Juku") and published as `sinijarv-juku`'s, the GitHub organization's. It is for people writing Juku programs, who have juku-sdk; most of it follows the assembler (the grammar, the `.dbg` files), and the emulator is only what it connects to. Its launch settings stay this server's (§12.7), so a change to them goes into both repositories. `vscode/package.py` makes `target/juku.vsix` there, with the SDK's version.
- **Contributes** the debug type `juku`, with snippets for launch and attach, and the language `juku-asm` for `.asm` files, with a TextMate grammar for the assembler's syntax. VS Code sets breakpoints only in languages named in an extension's `breakpoints` contribution, so the language is needed even without the colors.
  - *As built:* the language (`;` comments, no grammar yet), its breakpoints, `debugInfo` in the attach settings and a snippet with it.
  - *As built:* the TextMate grammar (`syntaxes/juku-asm.tmLanguage.json`): comments, labels with a colon or in the first column, strings, the assembler's directives, the 8080's mnemonics and registers, HIGH and LOW, numbers and operators. The launch settings with their descriptions, and snippets for launch and attach.
- **Connects** with a `DebugAdapterServer` for 127.0.0.1 and the port (`juku.port`, 5104 by default). If nothing listens, it starts the app with `--debug-server` (`open -a`, with `juku.app` naming the app, `Juku` by default) and tries again for 10 s.
  - *As built* on Windows: it starts `Juku.exe` with `--debug-server=<port>`. For `juku.app`'s default, `Juku`, it takes the copy the installer puts in `%LOCALAPPDATA%\Programs\Juku` if there is one, and otherwise leaves Windows to find `Juku` on the PATH; the setting can name another `Juku.exe` by its path. A Juku already running takes the port from that start (§11.8), so Accept Connections need not be ticked there, and the error after 10 s says only to set `juku.app`.
  - *As built:* it looks by connecting and closing at once. The server takes such a client and drops it at its next poll; a client that comes right behind it is not turned away for it, because a client that has gone is let go before a new one is looked at (`a_probe_and_then_a_client`). After 10 s the error says to tick Accept Connections in a Juku already running, or to quit it: `open -a` passes no arguments to an app that runs.
- **The devices** show as the Devices scope. If that proves clumsy, a webview panel fed by a custom request (`juku/devices`) can come later.
- **No packages.** The extension is one JavaScript file against VS Code's API, `extension.js`, with no build step: TypeScript would bring `typescript` and `@types/vscode` to check a few dozen lines. Nothing from npm is in the extension, nor needed to package it (below). `@vscode/debugadapter` is not needed, since the emulator is the adapter.
- **As built:** the debug type `juku` with an attach configuration and snippet, the `juku.port` setting, and a `DebugAdapterServer` for 127.0.0.1. F5 with no `launch.json` attaches. To try it without installing, `code --extensionDevelopmentPath="$PWD/vscode"` in juku-sdk; the extension's README says how.
- **Packaging:** `npm run package` makes a `.vsix`, installed with `code --install-extension`. Not on the Marketplace for now.
  - *As built:* `package.py` makes the `.vsix` without npm: a zip of the extension's files under `extension/`, with the manifest (`extension.vsixmanifest`) and the content types (`[Content_Types].xml`) that `vsce` writes. `code --extensions-dir <scratch folder> --install-extension` installed it. Packing came early, since run with `--extensionDevelopmentPath`, the extension is only in the window that option opens, and `.asm` files in any other window showed as plain text.
  - A user's `files.associations` mapping `*.asm` to another language wins over the extension's; the README says so.
- **Examples:** `demos/.vscode/` would be ignored as a dotfile, so the extension's README shows a `tasks.json` that assembles with `-g` and a `launch.json` that debugs the result.

### 12.9 Terminal client (postponed)
A full-screen `juku-debugger` in `debugger/cli/`: registers, disassembly, memory, the stack, breakpoints and the devices at once, with a command line. It would be one more DAP client of the same server, using custom requests for the devices. It may not be needed once VS Code works well.
- **Its terminal layer** is to be weighed when it comes. `ratatui` and `crossterm` would bring a tree of crates between them (to be counted then), for raw mode, the window size, key events and drawing a grid of cells. What it needs is a few hundred lines: `termios` and `ioctl(TIOCGWINSZ)` declared by hand for macOS and Linux, ANSI escapes, and a cell buffer drawn as its differences.

