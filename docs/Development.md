# Development

Building Juku from source, its tests, and debugging Juku programs with it.
How the emulator is built inside is in [Emulator_spec.md](Emulator_spec.md),
and what we have worked out about the real machine in
[Juku_machine_description.md](Juku_machine_description.md).

## Building

| | macOS | Windows |
|---|---|---|
| Builds | `./build.sh` → `target/Juku.app` | `build.cmd` → `target\windows\Juku.exe` |
| Runs on | macOS 14 or later, on the architecture it was built on | Windows 10 and 11, x64 |
| Needs | nightly Rust with `rust-src`; Xcode with its Metal Toolchain component (`xcodebuild -downloadComponent MetalToolchain`) | nightly Rust with `rust-src`; the Visual Studio Build Tools and the Windows SDK; Inno Setup 6 for the installer |

```sh
./build.sh                # release build, tuned for this computer
./build.sh debug          # unoptimized, with debug info
./build.sh dist           # release build for other Macs
./run.sh                  # run it, with its log
CODE_SIGN_IDENTITY=<hash> ./package.sh   # signed target/Juku-<version>.dmg
```

`build.cmd` takes the same `debug` and `dist`; on Windows `dist` makes a
zip and, with Inno Setup, an installer.

The build compiles Rust's standard library from source (`-Z build-std`),
to make the program smaller, so plain `cargo build --release` does not
work. `cargo test` does. Both need nightly Rust: `rust-toolchain.toml`
names a nightly by its date, so that every build uses the same compiler,
and `rustup` fetches it, with `rust-src`, on the first build. Moving to a
newer one is a change of that date, in juku-sdk's too.

The Rust code is one Cargo workspace: `core/` is the emulation core,
`debugger/dap/` the debug server, `frontends/windows/` the Windows app.
The macOS app is Objective-C, in `frontends/macos/`. Cargo's output goes
to `target/cargo`.

`package.sh` signs with `CODE_SIGN_IDENTITY`, a certificate in the
keychain (`security find-identity -v -p codesigning` lists them), and with
`NOTARY_PROFILE`, a `notarytool` keychain profile, notarizes the disk image
too. The first run asks to let the terminal control Finder, which lays out
the disk image's window.

## juku-sdk

The debug server is built on a crate of
[juku-sdk](https://github.com/sinijarv-juku/juku-sdk), `juku-json`, JSON
and the framing of DAP's messages, so the apps link it. The tests use three
more: the demo programs `juku-demos`, which the core's raster tests and benchmark run; the
assembler `juku-asm`, which the debug server's tests assemble the bounce
demo with; and the 8080's instruction table `juku-i8080`, which they check
the emulator's disassembler against, as they check its names against the
SDK's. They are *git dependencies* in `core/Cargo.toml` and
`debugger/dap/Cargo.toml`:

```toml
juku-asm = { git = "https://github.com/sinijarv-juku/juku-sdk", tag = "v0.3.0" }
```

Cargo clones the repository itself, at the tag, into its own cache, and
builds the crates from there; `Cargo.lock` records the commit. Nothing
needs to be cloned beside juku-emu, but the first build needs the network.
`build.sh` and `build.cmd` fetch the SDK too, for `juku-json`.

To move to a newer SDK, change the tag of every crate to the new release.

### Working on juku-sdk too

To have juku-emu use a copy of juku-sdk you are changing, clone both into
one folder and put a cargo configuration file in that folder, outside both
repositories:

```
juku/
├── .cargo/config.toml
├── juku-emu/
└── juku-sdk/
```

```toml
# juku/.cargo/config.toml
[patch."https://github.com/sinijarv-juku/juku-sdk"]
juku-asm = { path = "juku-sdk/crates/asm" }
juku-i8080 = { path = "juku-sdk/crates/i8080" }
juku-demos = { path = "juku-sdk/crates/demos" }
juku-json = { path = "juku-sdk/crates/json" }
```

Cargo reads configuration files in the folders above the one it runs in,
so this applies to juku-emu, and a relative path is taken from the folder
holding `.cargo`. Without the file, or in a clone elsewhere, the release
named by the tag is used.

Cargo keeps the versions `Cargo.lock` names, and the committed one names
the release. The local copy usually has another version (the next one with
`-dev`), so the patch is not used until the lock file is updated to take
it, in `juku-emu`:

```sh
cargo update juku-asm juku-i8080 juku-demos juku-json
```

Until then cargo builds the release, warns that the patch "was not used in
the crate graph", and adds `[[patch.unused]]` entries to `Cargo.lock`.
Check which copy is used with `cargo tree -p juku-dap -e dev -i juku-asm`.
Run the update again after juku-sdk's version changes, or after
`Cargo.lock` is restored from git.

While the patch is in use, `Cargo.lock` names the local copy instead of the
release. Leave that change out of commits. `build.sh` and `build.cmd` still
warn that the patch was not used: that is the standard library's own build
(`-Z build-std`), which the patch applies to as well, and it does no
harm.

## Tests

```sh
cargo test                                            # the core and the debug server
cargo test -p juku --test boot                        # one test file of the core
cargo test -p juku --test cpu_exercisers -- --ignored # 8080EXM, about 20 s
JUKU_TEST_DISKS=<folder> cargo test -p juku           # with the disk tests
cargo run -p juku --profile test --example bench      # emulation speed, × real time
```

The disk, mouse and other tests that boot from floppies need
`jukprog1.juk` and `jukgame1.juk`, the programs and games disks of MAME's
Juku software list, which are not in the repository; juku3000
(https://github.com/infoaed/juku3000) tells where to find them. The JBASIC
test needs `jbasic11.bin`, the JBASIC 1.1 expansion ROM.
`JUKU_TEST_DISKS` names the folder that holds them (a relative path is
taken from `core/`); without it those tests are skipped, and report as
passed.

The CPU tests run the classic 8080 test programs: TST8080, 8080PRE,
CPUTEST and 8080EXM. They are not ours, so they are not in the repository
either: each test downloads its program with `curl` the first time, from
[superzazu/8080](https://github.com/superzazu/8080) at a fixed commit, into
`target/cpu_tests`, where `cargo clean` leaves them. Without the network
they are skipped too.

## Continuous integration

GitHub Actions (`.github/workflows/ci.yml`) runs the tests and builds both
apps on every push to `main` and every pull request, except one that
changes only documentation (Markdown files and `docs/`): on a macOS runner
`cargo test` and `./build.sh dist`, on a Windows runner `cargo test` and
`build.cmd dist`, whose zip and installer are kept as the run's artifacts.
Neither is signed. To run it anyway, start it by hand:
`gh workflow run CI`.

## Debugging Juku programs

Juku is a debug server, on macOS and Windows: it speaks the Debug Adapter
Protocol (DAP) on `127.0.0.1`, port 5104, and VS Code connects to it
through juku-sdk's extension. A program can be launched from VS Code, stopped, stepped by
source line or by instruction, and its registers, memory, ports and
devices looked at; breakpoints take conditions, hit counts and log
messages, and watchpoints stop on a symbol's or a port's reads and writes.

1. Install juku-sdk's commands and its VS Code extension, from a clone of
   [juku-sdk](https://github.com/sinijarv-juku/juku-sdk):

   ```sh
   ./build.sh install                                   # juku-asm and the other commands
   python3 vscode/package.py && code --install-extension target/juku.vsix
   ```

2. Assemble with debug information, in juku-sdk's `demos/`:

   ```sh
   juku-asm -g bounce.asm          # bounce.bin and bounce.dbg
   ```

3. Launch it from VS Code, with a `.vscode/launch.json` like this:

   ```json
   {
       "version": "0.2.0",
       "configurations": [
           {
               "type": "juku",
               "request": "launch",
               "name": "Bounce",
               "program": "${workspaceFolder}/bounce.bin",
               "stopOnEntry": true
           }
       ]
   }
   ```

   If nothing listens on the port, the extension starts Juku with its
   debug server on. A Juku already running needs Settings → Machine →
   Debugger: Accept Connections. `--debug-server[=<port>]` on Juku's
   command line turns it on for one run, and
   `defaults write io.github.sinijarv-juku.juku-emu DebuggerPort <n>` moves
   it to another port.

   On Windows the extension starts `Juku.exe` from where the installer
   puts it, or the one its `juku.app` setting names, such as
   `D:\juku-emu\target\windows\Juku.exe`. A Juku already running there
   takes the port from the new start, so it needs no setting. The port
   is `DebuggerPort` in `%LOCALAPPDATA%\Juku\Juku.ini`, under
   `[Settings]`, changed while Juku is not running.

The extension's
[README](https://github.com/sinijarv-juku/juku-sdk/blob/main/vscode/README.md)
has the launch and attach settings, expressions and watchpoints; the
server is described in [Emulator_spec.md](Emulator_spec.md) §12, and the
Windows app's part of it in §11.8.

## Versions

The version is in `VERSION`, semantic (`MAJOR.MINOR.PATCH`), with `-dev`
while the next one is being made. While it is 0.x, a release may break
compatibility with earlier ones. Releases are tagged `vX.Y.Z`; the spec's
§3.3 says what each part means from 1.0.0.

## Releasing

1. Commit the version without `-dev`, `build: bump the version to X.Y.Z`,
   tag it `vX.Y.Z` and push both. The tag runs
   `.github/workflows/release.yml`, which builds the Windows zip and
   installer and makes a draft release with them, its notes with a TODO
   in place of the changes since the previous release.
2. On the Mac, build, sign and notarize the disk image, and add it to the
   draft:

   ```sh
   CODE_SIGN_IDENTITY=<hash> NOTARY_PROFILE=<profile> ./package.sh
   gh release upload vX.Y.Z target/Juku-X.Y.Z.dmg
   ```

3. Write the changes in the draft in place of its TODO, look it over and
   publish it on GitHub (or `gh release edit vX.Y.Z --notes-file notes.md
   --draft=false`).
4. Commit the next version, `build: start version X.Y+1.0-dev`.
