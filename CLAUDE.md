# Juku emulator

A native emulator of the Juku E5101/E5104, an Estonian school computer from
the late 1980s (Intel 8080 class CPU, RomBios in ROM, EKDOS on floppies), for
macOS and Windows.

- `core/` is the emulation core: a Rust crate (`juku`) with no dependencies and
  no OS APIs. The macOS app talks to it only through the C ABI in
  `core/src/ffi.rs`, and to the debug server through
  `debugger/dap/src/ffi.rs`.
- `debugger/` is the guest debugger (spec §12): `dap/` is the crate
  `juku-dap`, a DAP server over TCP with no dependencies but the core. The
  VS Code extension that connects to it is juku-sdk's (`../juku-sdk/vscode`,
  the extension `juku`): its launch settings are what the server takes, so
  a change to them goes into both. The Mac app links `juku-dap`'s static
  library, which holds the core and its C ABI too; the Windows app uses
  the crate's Rust API, as it uses the core's.
- The assembler, `juku-asm`, is part of juku-sdk, a repository of its own
  beside this one (`../juku-sdk`, crate `crates/asm`): a command, and a
  library the debug server's tests assemble the demos with. The server
  takes it as a git dependency on juku-sdk's GitHub repository at a
  release tag; `../.cargo/config.toml`, outside both repositories, patches
  in the copy beside this one (docs/Development.md, "juku-sdk").
- `frontends/macos/` is the macOS layer in Objective-C (`src/`), with Metal
  for the CRT display, and its own resources (`resources/`: the plist, the
  icon, `InfoPlist.strings`).
- `frontends/windows/` is the Windows layer: Rust with the `windows` crate,
  Win32 and Direct3D 12, calling the core's Rust API directly. The plan,
  the decisions taken and the as-built notes are in spec §11. `frontends/windows/thumbnail/` is Explorer's thumbnail
  handler for snapshots, a DLL. The macOS app is the reference for
  behavior.
- `resources/` holds what the front-ends share: the ROM images in `roms/`
  (`ekta31.bin`, `ekta37.bin`, `jmon33.bin`, 16 KB each; `ekta37.bin` is
  the default) and the UI strings in `strings/`. The user can also pick a ROM
  file of their own, and an expansion ROM such as JBASIC, which is not
  bundled.

## Documentation

Read the relevant section before changing behavior, and update it in the same
change. The documents are kept in step with the code. Write them as a
professional technical writer would: neutral and factual, with no hype or
marketing tone, in American spelling (behavior, color, center, gray,
catalog, startup). Nothing has been tried on a real machine, so do not
state that the emulation matches the hardware: say what it is based on,
or that programs should behave as they would there.

- `docs/Emulator_spec.md`: how the emulator is built (layout, build, core
  architecture, FFI, macOS layer, rendering, tests, decisions taken, the
  Windows front-end in §11).
- `docs/Juku_machine_description.md`: how the real machine works (memory and
  I/O maps, devices, keyboard, video, boot, RomBios interface). The spec cites
  it as "MD §n".
- `README.md`: for people using the app: its features, installation, use
  (the menus, the shortcuts on both systems) and the screenshots in
  `docs/images/`. A change users see goes in here too.
- `docs/Development.md`: building, the tests, juku-sdk as a git dependency
  and debugging Juku programs; for people working on the code.

What the real machine shows (its ROMs, disks, flux dumps, documents) always
comes before MAME, which is not always right. MAME's `juku` driver is the
behavioral reference only for what nothing real settles.

## Commands

```sh
./build.sh            # release build tuned for this machine -> target/Juku.app
./build.sh debug      # unoptimized build with debug info
./build.sh dist       # release build for other Macs
./run.sh              # run the built app with its log stream attached

cargo test                                                 # all tests: the core's and the debug server's
cargo test -p juku --test boot                             # one test file of the core
cargo test -p juku --test cpu_exercisers -- --ignored      # 8080EXM, ~20 s; the test programs are downloaded
cargo test -p juku-dap                                     # the debug server, with a DAP client over a socket
python3 ../juku-sdk/vscode/package.py && code --install-extension ../juku-sdk/target/juku.vsix   # the VS Code extension, juku-sdk's
cargo run -p juku --profile test --example bench           # emulation speed, × real time
cargo run -p juku --release -Z build-std=std --target $(rustc -vV | sed -n 's/^host: //p') --example bench   # the same as the app is built: use this to compare before and after

CODE_SIGN_IDENTITY=<hash> ./package.sh   # signed target/Juku-<version>.dmg, its window laid out by Finder (spec §3.4)

../juku-sdk/build.sh install                               # the SDK's commands, juku-asm too, on the PATH
juku-asm <source.asm> <out.bin> [-l out.lst] [-s out.sym] [-g out.dbg]   # assemble; -g writes debug information (spec §12.6)
juku-disk ls|get|put|rm|new <image> ...                     # files on EKDOS disk images: juku-sdk's, ../juku-sdk/target/bin
juku-dis <file> [<start> [<end>]] [--base 0] [-s] [-i rombios.inc]   # disassemble: juku-sdk's; base 100 (COM) by default, -s source
python3 tools/scpdisk.py <image.scp> [-t CYL.SIDE | -o out.juk]   # decode a flux dump: per-track summary, a track's bytes, or a sector image
```

On Windows:

```text
build.cmd             # release build tuned for this PC -> target\windows\Juku.exe, JukuThumbnail.dll
build.cmd debug       # unoptimized console program: its log shows in the terminal
build.cmd dist        # release build for other PCs, target\Juku-<version>-windows-x64.zip,
                      # and with Inno Setup 6 installed, ...-setup.exe
powershell -ExecutionPolicy Bypass -File frontends\windows\resources\make_artwork.ps1   # Juku.ico and the installer's images from the Mac icon
..\juku-sdk\build.cmd install                 # the SDK's commands, juku-asm too, on the PATH
```

- The build needs nightly Rust (`-Z build-std`, `panic = "immediate-abort"`)
  with `rust-src`, pinned by date in `rust-toolchain.toml`, the same date
  as juku-sdk's: change both together; on macOS Xcode's Metal Toolchain component, on Windows the
  Visual Studio Build Tools (the MSVC linker) and the Windows SDK (`rc.exe`).
  The Windows installer needs Inno Setup 6 (`winget install JRSoftware.InnoSetup`);
  without it `build.cmd dist` makes the zip only.
- Plain cargo cannot use the release profile, because `immediate-abort` needs
  `build-std`. Run examples with `--profile test`, for example
  `cargo run -p juku --profile test --example boot_dump -- <seconds> <out.pbm>`.
- The Rust crates are one workspace, the root `Cargo.toml`: one
  `Cargo.lock`, the profiles there and in no crate's manifest.
  `.cargo/config.toml` puts cargo's output in `target/cargo`, so `cargo
  clean` leaves the app and the packages in `target/` alone. Plain cargo
  takes the core and the debug server; the Windows crates build on Windows
  only, named with `-p` (`build.cmd` does).
- **rust-analyzer on a Mac** checks the whole workspace (`cargo check
  --workspace`), so it shows errors from the Windows crates and the
  `windows` crate's sources. Set `rust-analyzer.check.workspace = false` in
  the editor: it then checks only the package of the file being saved. To
  work on the Windows crates on a Mac, set `rust-analyzer.cargo.target =
  "x86_64-pc-windows-msvc"`; their build script still cannot run there.

## Tests and disk images

- The disk, drive sound, mouse, ZOO and RDEM tests boot from `jukprog1.juk` and
  `jukgame1.juk`, and the JBASIC test needs `jbasic11.bin`. These files are
  not in the repository. `JUKU_TEST_DISKS` names the directory holding them, and the
  tests are skipped when it is not set.
- A skipped test still reports as passed. After touching the floppy
  controller, disk, mouse, video or memory map code, run with the variable
  set. If local
  copies exist in `floppies/` (git-ignored):
  `JUKU_TEST_DISKS=../floppies cargo test -p juku`
  (a relative path is resolved from `core/`).

## Things that are easy to get wrong

- **Single translation unit.** All Objective-C is compiled through
  `frontends/macos/src/Main.m`. A new `.m` file must be `#import`ed there
  (and start with `#pragma once`); `build.sh` does not list source files.
  The exception is `frontends/macos/src/thumbnail/`, the Quick Look
  extension, which `build.sh` builds as a binary of its own.
- **Quick Look keeps its extension list.** After the thumbnail extension is
  first installed or renamed, Finder shows snapshot thumbnails only after
  logging out and in: the thumbnail agent cannot be restarted (SIP).
- **Resources are hard-linked into the bundle.** `build.sh` re-links any
  ROM, strings or `.lproj` file that was replaced or added, so a build picks
  up every change. A resource deleted from `resources/` or
  `frontends/macos/resources/` stays in `target/Juku.app` until the bundle is
  deleted and rebuilt.
- **Two UI languages.** Every user-visible string goes through `localized()`
  and needs an entry in both `resources/strings/en.strings` and
  `resources/strings/et.strings`. The front-ends share these files; `build.sh`
  puts each into the bundle as `<language>.lproj/Localizable.strings`.
- **The ROM images are not ours.** The MIT license covers the code and docs
  only; see `resources/roms/README.md`. Do not add other third-party ROMs,
  disk images or software to the repository.
- **Assembly source is lower case**: mnemonics, registers, directives,
  labels and hex digits (`mvi a, 0ffh`), a space after each operand comma;
  constants made with `equ` are upper case (`TTO equ 0ffd9h`). Names are
  case-sensitive in juku-asm, so a use must match its definition. RomBios's
  names and the ports come from juku-sdk's include files
  (`include 'rombios.inc'`, `'ports.inc'`). The debug server's tests find
  `demos/bounce.asm`'s lines by their text (`bounce_line`).
- **System symbols are juku-sdk's.** `resources/symbols/rombios.dbg` and
  `ekdos.dbg` hold the names and values of juku-sdk's `include/rombios.inc`
  and `ekdos.inc`; the test `system_symbols_are_the_sdks` checks it. Change
  both together.
- **Dotfiles are ignored.** `.gitignore` ignores everything starting with a
  dot except itself, `.gitattributes`, `.github/` and `.cargo/`; another
  dotfile needs its own exception. `.cargo/config.toml` sets only the target
  directory: the build flags stay in `build.sh` and `build.cmd`, since they
  depend on the mode and would otherwise apply to `cargo test` too.
- **Two copies of the shaders.** `frontends/macos/src/Shaders.metal` and
  `frontends/windows/src/Shaders.hlsl` hold the same passes with the same
  constants; a change to the picture goes into both. The HLSL is
  compiled by `build.rs` with warnings as errors: `line`, `flat` and `out`
  are HLSL keywords, and fxc takes an early return in a function for a use
  of an uninitialized value.
- **The Windows artwork is made, not drawn.** `frontends/windows/resources/Juku.ico`
  and the installer's `Wizard*Image-*.png` come from the Mac's `Juku.icon`
  through `make_artwork.ps1`; after changing the Mac icon, run it and commit
  what it writes.
- **Explorer holds the thumbnail DLL.** Once it has drawn snapshot
  thumbnails, `target\windows\JukuThumbnail.dll` stays loaded for a while
  and `build.cmd` cannot replace it until it is let go. The DLL's class ID is
  written in `frontends/windows/thumbnail/src/lib.rs` and in
  `frontends/windows/src/file_types.rs`; keep them the same.
- **Windows-only strings** go at the end of both `.strings` files, under the
  `/* Windows only */` comment. Their keys are new; the shared keys are the
  Mac's English titles. A Windows menu item also needs its title with the
  underlined letter marked, keyed `&` plus the English title
  (`"&Machine" = "Ma&chine";`), in the menu-title block at the end. The
  menu bar's letters must not be Alt shortcut letters (spec §11.3).
- **Windows dialogs have an icon.** Every alert, message box and task
  dialog shows the icon for what it says: the error icon for something
  that could not be done, the question mark for a question
  (`IDI_QUESTION`; task dialogs have no `TD_` constant for it, see
  `disks::ask_to_save`), the warning or information icon otherwise, and
  the app's icon for About.

## Identifiers

The version is the file `VERSION`, and nowhere else: the app's bundle, the
Quick Look extension and the Windows exe and installer take it from there
when built. The crates have none (Cargo's 0.0.0). It is a semantic
version, `MAJOR.MINOR.PATCH`, with `-dev` while the next version is being
made (0.5.0-dev after release 0.4.0). The OS fields take numbers only:
they get `MAJOR.MINOR.PATCH` and the build number, 0 for `-dev` and 1 for a
release; the app shows the version, a `-dev` build with its commit. While
it is 0.x, a release need not be compatible with earlier ones; spec §3.3
says what each part means from 1.0.0. To release, drop `-dev` in a commit
of its own, `build: bump the version to X.Y.Z`, and tag it `vX.Y.Z`; the
next commit starts `X.Y+1.0-dev`. The tag's workflow drafts the GitHub
release with the Windows files; the signed macOS disk image is added from
the Mac (docs/Development.md, "Releasing").

The bundle identifier is `io.github.sinijarv-juku.juku-emu`, from the GitHub
organization `sinijarv-juku`, and the document types use the same prefix
(`io.github.sinijarv-juku.juku-emu.disk-image`, `.raw-disk-image`,
`.snapshot`). It names the app's settings, so it does not change once
released. Windows' ProgIDs allow no hyphens and at most 39 characters:
they are `SinijarvJuku.JukuEmu.DiskImage` and `.Snapshot`.

## Commits

- Subject line in the Neovim repository's style: `type(scope): subject`, with
  type one of `build ci docs feat fix perf refactor revert test`, an optional
  scope, imperative voice, under 72 characters.
- A single line most of the time. Add a body only when something about the
  change is not obvious.
- No AI attribution: no `Co-Authored-By` or "generated with" lines.
