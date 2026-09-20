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

## Privileges

* Root: everything runs directly; flatpak installs system-wide.
* Normal user: backends needing root (apt, pacman, dnf, ...) run through `sudo`/`doas`; flatpak
  installs with `--user`.
* `--user` / `mode = "user"`: user-only. Backends needing root are skipped, never escalated.
* Backends that refuse root (brew, paru, yay, pacstall, scoop) are skipped when running as root.

## Config

Directory: `--config-dir` / `$ZU_CONFIG_DIR`, else `$XDG_CONFIG_HOME/zu` or `~/.config/zu`
(`%APPDATA%\zu` on Windows). All files are optional; `zu config init` writes starters.

`config.toml` — settings:

```toml
[general]
mode = "auto"       # or "user"
escalate = "auto"   # "sudo" | "doas" | "never"
fallback = true
assume_yes = false

[cli]
style = "apt"       # or "pacman"
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

Only the Arch/pacman/paru/flatpak/nix paths have been exercised on real hardware. Command
templates for the other backends are in `src/backends.rs` and are unit-tested for shape, not run.
