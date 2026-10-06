# zu — Zex's Universal

One pure-Rust front-end for every package manager. It detects the platform, tries the platform's
own manager first, then falls back through the rest in an order you control.

```
zu install hyprland          # native manager first, then fallbacks
zu remove hyprland           # asks each backend who actually has it
zu purge hyprland            # remove plus config/data (same as remove where a backend has no purge)
zu search foo | info foo | list | update | upgrade
zu backends                  # detected backends in priority order
zu doctor                    # platform, privileges, config, missing backends
zu config init              # write starter config files listing only detected backends
```

Global flags: `-y` non-interactive, `-n` dry run, `-b ID` force one backend (ignores package
rules), `--skip ID`, `--user` user-only mode, `--no-fallback`, `--config-dir DIR`.

### Command style

Two dialects, chosen by `style` under `[cli]` in `config.toml` (default `apt`). `ZU_STYLE=pacman`
overrides it for one run.

| apt style (default) | pacman style | |
|---|---|---|
| `zu install foo` | `zu -S foo` | install |
| `zu remove foo` | `zu -R foo` | remove |
| `zu purge foo` | `zu -Rn foo` / `-Rns` | remove + config/data |
| `zu search foo` | `zu -Ss foo` | search |
| `zu info foo` | `zu -Si foo` | package details |
| `zu update` | `zu -Sy` | refresh indexes |
| `zu upgrade` | `zu -Syu` / `-Su` | upgrade everything |
| `zu list` | `zu -Q` | list installed |
| `zu backends`, `zu doctor`, `zu config path\|init` | same | zu's own commands |

Pacman style follows pacman's flag meanings, so `-y` is refresh and `-n` is nosave there. Use
`--noconfirm` for non-interactive and `-p`/`--print` for a dry run; `-b`, `--skip`, `--user`,
`--no-fallback` and `--config-dir` are unchanged.

## Installing

Pre-built `.deb` packages (amd64, arm64) are attached to each
[GitHub release](https://github.com/Zexolver/zu/releases). They are statically linked (musl), so
they have no runtime library dependencies and work on any Debian/Ubuntu-family release:

```
sudo apt install ./zu_0.1.0_amd64.deb   # or arm64
```

The package also drops a system-wide default at `/etc/zu/sources.toml`: try apt, then pacstall,
then flatpak, then give up (snap/nix/brew are disabled there on purpose, so the chain stays short
out of the box). It's a conffile, so `dpkg`/`apt` leave your edits alone across upgrades. A
per-user `~/.config/zu/sources.toml` (`zu config init`) always overrides it — see
[Config](#config) below.

Otherwise, `cargo install --path .` from a clone, or build manually (see `scripts/build-deb.sh`
for the exact cross-compilation steps for both architectures).

## Backends

| Platform | Backends (default order) |
|---|---|
| Debian/Ubuntu | apt, pacstall, flatpak, snap, nix, brew |
| Arch | pacman, paru, yay, flatpak, snap, nix, brew |
| Fedora/RHEL, openSUSE, Alpine, Void, Gentoo, Solus | dnf, zypper, apk, xbps, emerge, eopkg + flatpak, snap, nix, brew |
| macOS | brew, port, nix |
| Windows | winget, scoop, choco |
| Termux | pkg, apt, dpkg (local `.deb` files) |
| FreeBSD/DragonFly, NetBSD, OpenBSD | pkg, pkgin, pkg_add |

Only backends that apply to the platform *and* are on `PATH` are ever used or written to the config.

### `zu update`

`update` refreshes every configured backend, not just the first one: each backend with its own
refresh step gets one, and a backend without one (pacman, pacstall, paru/yay, snap, choco, nix,
pkg_add, ...) prints a one-line reason instead of silently doing nothing — those are combined
refresh+upgrade (or have no index at all), so there's nothing separate to run; `zu upgrade` covers
them. Backends that refuse to run as root (pacstall, paru/yay, brew, scoop) are skipped under
`sudo zu update` the same as everywhere else — run it as yourself instead and zu escalates only the
backends that need it.

`[ui] style` in `config.toml` picks how this (and nothing else, for now) is shown:
* `"pretty"` (default): a spinner per backend plus an overall progress bar, each backend's own
  output captured rather than printed (shown on failure), ending in `Done!` and, if
  `list_upgradable = true` (default), every package every backend found pending an upgrade.
* `"raw"`: each backend's own output verbatim, same as `install`/`upgrade`/etc., with a plain
  `[i/n] done` line after each one instead of the progress bar.

Only `apt` currently knows how to list what it would upgrade (`apt list --upgradable`); other
backends still get refreshed and counted toward the `i/n` progress, just without a package count.

## Privileges

* Root: everything runs directly; flatpak installs system-wide.
* Normal user: backends needing root (apt, pacman, dnf, ...) run through `sudo`/`doas`; flatpak
  installs with `--user`.
* `--user` / `mode = "user"`: user-only. Backends needing root are skipped, never escalated.
* Backends that refuse root (brew, paru, yay, pacstall, scoop) are skipped when running as root.

## Config

Directory: `--config-dir` / `$ZU_CONFIG_DIR`, else `$XDG_CONFIG_HOME/zu` or `~/.config/zu`
(`%APPDATA%\zu` on Windows). All files are optional; `zu config init` writes starters.

A packaged install (like the `.deb`) may also drop defaults in `/etc/zu` (`%ProgramData%\zu` on
Windows; nothing on Termux). Those are used only for files missing from your own directory above —
any file you have there wins outright, with no merging. `zu config path` and `zu doctor` show which
directory ended up providing each file.

`config.toml` — settings:

```toml
[general]
mode = "auto"       # or "user"
escalate = "auto"   # "sudo" | "doas" | "never"
fallback = true
assume_yes = false

[cli]
style = "apt"       # or "pacman"

[ui]
style = "pretty"         # or "raw"; see "zu update" above
list_upgradable = true
```

`sources.toml` — which package managers, in what order:

```toml
order = ["flatpak", "pacstall", "apt"]   # listed first; the rest follow in default order
disabled = ["snap"]

[options.pacman]
install_args = ["--asdeps"]
```

`packages.toml` — per-package rules:

```toml
[hyprland]
skip = ["apt"]                  # skip apt, use the normal order for the rest
# prefer = ["pacstall"]         # try these first, then the normal order
# only = ["pacstall"]           # use nothing else, in this order

[hyprland.names]                # name differs per backend
pacstall = "hyprland-git"
```

## Status

Only the Arch/pacman/paru/flatpak/nix paths, the `.deb` packaging, and `zu update`'s apt/pacstall
path (simulated: apt and pacstall aren't installable on the dev machine) have been exercised on
real or emulated hardware. Command templates for the other backends are in `src/backends.rs` and
are unit-tested for shape, not run.
