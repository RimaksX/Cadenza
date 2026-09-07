# Building the installer

Cadenza ships as an MSI, which PROJECT_MASTER 1.2 fixes as the distribution
format. This directory holds everything the installer is made of except the
program itself.

```
cadenza.wxs          the manifest — what is installed, where, and under what name
assets/banner.bmp    493x58, the strip across the top of every page but the first
assets/dialog.bmp    493x312, the welcome and the farewell
assets/license.rtf   the GPL, generated from LICENSE at the repository root
out/                 where the MSI lands; ignored by git
```

## Once

WiX is a .NET tool and is not part of the Rust toolchain:

```
dotnet tool install --global wix
wix extension add -g WixToolset.UI.wixext
```

## Every time

Build the program first. The installer packages what is in `target/release`,
and a debug build is four times the size and slower.

```
cargo build --release
```

Then, from this directory:

```
wix build cadenza.wxs -ext WixToolset.UI.wixext -o out/Cadenza-0.1.0.msi
```

To package a build from somewhere else, name it:

```
wix build cadenza.wxs -ext WixToolset.UI.wixext -d Binaries=<path> -o out/Cadenza-0.1.0.msi
```

## What it does on the listener's machine

**Installs for one person and never asks for administrator.** The program goes
to `%LOCALAPPDATA%\Programs\Cadenza`, beside the data it already keeps in
`%LOCALAPPDATA%\Cadenza`. A player has no use for a permission over the whole
machine, and asking for one is how a small program starts looking like a large
one.

**Leaves the library alone when it is uninstalled.** The database, the artwork
cache and the log stay where they are. Somebody who reinstalls next week expects
their library and their history to still be there, and a player that deletes a
year of listening because it was asked to remove a program has answered a
question nobody put to it. Removing that folder by hand is one gesture; getting
it back is none.

**Carries the program and not the map of it.** Only `cadenza.exe` is named in
the manifest, so the `.pdb` that the release build leaves beside it cannot
travel by accident.

## Two things it does not do yet

**It is not signed.** An unsigned installer meets a listener with SmartScreen's
blue "Windows protected your PC" panel, and no amount of design inside the
wizard answers that. A code-signing certificate costs money and requires the
publisher to be verified; until there is one, the warning is what everybody
sees first. This matters more to how the installation feels than any bitmap in
this directory.

**The version is written in two places.** `Cargo.toml` and `cadenza.wxs` both
say `0.1.0`, and nothing checks that they agree. A release worth automating is
a release where that is read from one of them.
