//! Static table of every supported package manager.
//!
//! A command is an argv template whose first element is the executable. `{yes}` expands to the
//! backend's non-interactive flags (only with `--yes`), `{user}` to its per-user scope flags
//! (only when not operating system-wide). Package names are appended at the end.

use crate::platform::{Os, Platform};

/// Default priority tier: the platform's own manager, then community/fallback ones, then the
/// cross-distro ones. Ties keep table order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Native,
    Community,
    Universal,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Native => "native",
            Kind::Community => "community",
            Kind::Universal => "universal",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priv {
    /// Mutating operations need root.
    Root,
    /// Never needs privileges.
    User,
    /// System-wide as root, per-user (`Backend::user` flags) otherwise.
    Flexible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Install,
    Remove,
    /// Remove plus configuration/data. Falls back to `remove` on backends with no such concept.
    Purge,
    Search,
    Info,
    Refresh,
    Upgrade,
    List,
}

impl Op {
    pub fn mutating(self) -> bool {
        matches!(self, Op::Install | Op::Remove | Op::Purge | Op::Refresh | Op::Upgrade)
    }
}

/// How to tell whether a package is installed.
#[derive(Clone, Copy, Debug)]
pub enum Probe {
    /// `argv + pkg`; installed iff it exits 0.
    Exit(&'static [&'static str]),
    /// `argv + pkg`; installed iff it exits 0 and stdout contains the marker.
    Marker(&'static [&'static str], &'static str),
    /// `argv` (no package); installed iff stdout mentions the package name as a whole word.
    Listing(&'static [&'static str]),
}

pub type Cmd = Option<&'static [&'static str]>;

pub struct Backend {
    pub id: &'static str,
    pub name: &'static str,
    /// Executable whose presence on `PATH` means the backend is installed.
    pub bin: &'static str,
    pub kind: Kind,
    pub os: &'static [Os],
    /// Linux only: os-release `ID`/`ID_LIKE` values. Empty means any.
    pub distros: &'static [&'static str],
    pub privilege: Priv,
    /// The tool itself errors out when run as root (brew, AUR helpers, scoop, ...).
    pub refuses_root: bool,
    pub yes: &'static [&'static str],
    pub user: &'static [&'static str],
    /// Prepended to package names on install (nix wants `nixpkgs#name`).
    pub install_prefix: &'static str,
    /// Installs only local files with this extension, never names (dpkg).
    pub local_ext: Option<&'static str>,
    /// Backends in one group share a database; `upgrade` runs only the first one found.
    pub group: &'static str,
    pub refresh_before_upgrade: bool,
    pub install: Cmd,
    pub remove: Cmd,
    pub purge: Cmd,
    pub search: Cmd,
    pub info: Cmd,
    pub refresh: Cmd,
    pub upgrade: Cmd,
    pub list: Cmd,
    pub probe: Option<Probe>,
    /// Where to get it, shown by `doctor` when it is applicable but missing.
    pub hint: &'static str,
}

impl Backend {
    pub fn cmd(&self, op: Op) -> Cmd {
        match op {
            Op::Install => self.install,
            Op::Remove => self.remove,
            Op::Purge => self.purge.or(self.remove),
            Op::Search => self.search,
            Op::Info => self.info,
            Op::Refresh => self.refresh,
            Op::Upgrade => self.upgrade,
            Op::List => self.list,
        }
    }
}

const BASE: Backend = Backend {
    id: "",
    name: "",
    bin: "",
    kind: Kind::Native,
    os: &[],
    distros: &[],
    privilege: Priv::Root,
    refuses_root: false,
    yes: &[],
    user: &[],
    install_prefix: "",
    local_ext: None,
    group: "",
    refresh_before_upgrade: false,
    install: None,
    remove: None,
    purge: None,
    search: None,
    info: None,
    refresh: None,
    upgrade: None,
    list: None,
    probe: None,
    hint: "",
};

const DPKG_INSTALLED: Probe = Probe::Marker(&["dpkg-query", "-W", "-f=${Status}"], "install ok installed");

const BREW: Backend = Backend {
    id: "brew",
    name: "Homebrew",
    bin: "brew",
    kind: Kind::Universal,
    os: &[Os::Linux],
    privilege: Priv::User,
    refuses_root: true,
    refresh_before_upgrade: true,
    install: Some(&["brew", "install"]),
    remove: Some(&["brew", "uninstall"]),
    search: Some(&["brew", "search"]),
    info: Some(&["brew", "info"]),
    refresh: Some(&["brew", "update"]),
    upgrade: Some(&["brew", "upgrade"]),
    list: Some(&["brew", "list"]),
    probe: Some(Probe::Exit(&["brew", "list"])),
    hint: "https://brew.sh",
    ..BASE
};

pub static ALL: &[Backend] = &[
    // ---- Linux: distro-native ----
    Backend {
        id: "apt",
        name: "APT",
        bin: "apt-get",
        os: &[Os::Linux],
        distros: &["debian", "ubuntu"],
        yes: &["-y"],
        refresh_before_upgrade: true,
        install: Some(&["apt-get", "install", "{yes}"]),
        remove: Some(&["apt-get", "remove", "{yes}"]),
        purge: Some(&["apt-get", "purge", "{yes}"]),
        search: Some(&["apt-cache", "search"]),
        info: Some(&["apt-cache", "show"]),
        refresh: Some(&["apt-get", "update"]),
        upgrade: Some(&["apt-get", "upgrade", "{yes}"]),
        list: Some(&["dpkg-query", "-W"]),
        probe: Some(DPKG_INSTALLED),
        ..BASE
    },
    Backend {
        id: "dnf",
        name: "DNF",
        bin: "dnf",
        os: &[Os::Linux],
        distros: &["fedora", "rhel", "centos"],
        yes: &["-y"],
        install: Some(&["dnf", "install", "{yes}"]),
        remove: Some(&["dnf", "remove", "{yes}"]),
        search: Some(&["dnf", "search"]),
        info: Some(&["dnf", "info"]),
        refresh: Some(&["dnf", "makecache"]),
        upgrade: Some(&["dnf", "upgrade", "{yes}"]),
        list: Some(&["dnf", "list", "--installed"]),
        probe: Some(Probe::Exit(&["rpm", "-q"])),
        ..BASE
    },
    Backend {
        id: "zypper",
        name: "Zypper",
        bin: "zypper",
        os: &[Os::Linux],
        distros: &["suse", "opensuse"],
        yes: &["-n"],
        install: Some(&["zypper", "{yes}", "install"]),
        remove: Some(&["zypper", "{yes}", "remove"]),
        search: Some(&["zypper", "search"]),
        info: Some(&["zypper", "info"]),
        refresh: Some(&["zypper", "refresh"]),
        upgrade: Some(&["zypper", "{yes}", "update"]),
        list: Some(&["zypper", "search", "--installed-only"]),
        probe: Some(Probe::Exit(&["rpm", "-q"])),
        ..BASE
    },
    Backend {
        id: "pacman",
        name: "pacman",
        bin: "pacman",
        os: &[Os::Linux],
        distros: &["arch"],
        yes: &["--noconfirm"],
        // No standalone refresh: `pacman -Sy` without `-u` risks partial upgrades.
        install: Some(&["pacman", "-S", "--needed", "{yes}"]),
        remove: Some(&["pacman", "-Rs", "{yes}"]),
        purge: Some(&["pacman", "-Rns", "{yes}"]),
        search: Some(&["pacman", "-Ss"]),
        info: Some(&["pacman", "-Si"]),
        upgrade: Some(&["pacman", "-Syu", "{yes}"]),
        list: Some(&["pacman", "-Q"]),
        probe: Some(Probe::Exit(&["pacman", "-Qi"])),
        ..BASE
    },
    Backend {
        id: "apk",
        name: "apk",
        bin: "apk",
        os: &[Os::Linux],
        distros: &["alpine"],
        refresh_before_upgrade: true,
        install: Some(&["apk", "add"]),
        remove: Some(&["apk", "del"]),
        purge: Some(&["apk", "del", "--purge"]),
        search: Some(&["apk", "search"]),
        info: Some(&["apk", "info"]),
        refresh: Some(&["apk", "update"]),
        upgrade: Some(&["apk", "upgrade"]),
        list: Some(&["apk", "info"]),
        probe: Some(Probe::Exit(&["apk", "info", "-e"])),
        ..BASE
    },
    Backend {
        id: "xbps",
        name: "XBPS",
        bin: "xbps-install",
        os: &[Os::Linux],
        distros: &["void"],
        yes: &["-y"],
        install: Some(&["xbps-install", "{yes}"]),
        remove: Some(&["xbps-remove", "-R", "{yes}"]),
        search: Some(&["xbps-query", "-Rs"]),
        info: Some(&["xbps-query", "-R"]),
        refresh: Some(&["xbps-install", "-S"]),
        upgrade: Some(&["xbps-install", "-Su", "{yes}"]),
        list: Some(&["xbps-query", "-l"]),
        probe: Some(Probe::Exit(&["xbps-query"])),
        ..BASE
    },
    Backend {
        id: "emerge",
        name: "Portage",
        bin: "emerge",
        os: &[Os::Linux],
        distros: &["gentoo"],
        refresh_before_upgrade: true,
        install: Some(&["emerge", "--noreplace"]),
        remove: Some(&["emerge", "--depclean"]),
        search: Some(&["emerge", "--search"]),
        refresh: Some(&["emerge", "--sync"]),
        upgrade: Some(&["emerge", "--update", "--deep", "--newuse", "@world"]),
        list: Some(&["qlist", "-I"]),
        probe: Some(Probe::Exit(&["qlist", "-I"])),
        ..BASE
    },
    Backend {
        id: "eopkg",
        name: "eopkg",
        bin: "eopkg",
        os: &[Os::Linux],
        distros: &["solus"],
        yes: &["-y"],
        refresh_before_upgrade: true,
        install: Some(&["eopkg", "install", "{yes}"]),
        remove: Some(&["eopkg", "remove", "{yes}"]),
        search: Some(&["eopkg", "search"]),
        info: Some(&["eopkg", "info"]),
        refresh: Some(&["eopkg", "update-repo"]),
        upgrade: Some(&["eopkg", "upgrade", "{yes}"]),
        list: Some(&["eopkg", "list-installed"]),
        ..BASE
    },
    // ---- Linux: community / fallback ----
    Backend {
        id: "pacstall",
        name: "Pacstall",
        bin: "pacstall",
        kind: Kind::Community,
        os: &[Os::Linux],
        distros: &["debian", "ubuntu"],
        privilege: Priv::User,
        refuses_root: true,
        yes: &["-P"],
        install: Some(&["pacstall", "{yes}", "-I"]),
        remove: Some(&["pacstall", "{yes}", "-R"]),
        search: Some(&["pacstall", "-S"]),
        info: Some(&["pacstall", "-Si"]),
        upgrade: Some(&["pacstall", "{yes}", "-Up"]),
        list: Some(&["pacstall", "-L"]),
        probe: Some(DPKG_INSTALLED),
        hint: "https://pacstall.dev",
        ..BASE
    },
    Backend {
        id: "paru",
        name: "paru (AUR)",
        bin: "paru",
        kind: Kind::Community,
        os: &[Os::Linux],
        distros: &["arch"],
        privilege: Priv::User,
        refuses_root: true,
        yes: &["--noconfirm"],
        group: "aur",
        install: Some(&["paru", "-S", "--needed", "{yes}"]),
        remove: Some(&["paru", "-Rs", "{yes}"]),
        purge: Some(&["paru", "-Rns", "{yes}"]),
        search: Some(&["paru", "-Ss"]),
        info: Some(&["paru", "-Si"]),
        upgrade: Some(&["paru", "-Sua", "{yes}"]),
        list: Some(&["paru", "-Qm"]),
        probe: Some(Probe::Exit(&["pacman", "-Qi"])),
        hint: "https://github.com/Morganamilo/paru",
        ..BASE
    },
    Backend {
        id: "yay",
        name: "yay (AUR)",
        bin: "yay",
        kind: Kind::Community,
        os: &[Os::Linux],
        distros: &["arch"],
        privilege: Priv::User,
        refuses_root: true,
        yes: &["--noconfirm"],
        group: "aur",
        install: Some(&["yay", "-S", "--needed", "{yes}"]),
        remove: Some(&["yay", "-Rs", "{yes}"]),
        purge: Some(&["yay", "-Rns", "{yes}"]),
        search: Some(&["yay", "-Ss"]),
        info: Some(&["yay", "-Si"]),
        upgrade: Some(&["yay", "-Sua", "{yes}"]),
        list: Some(&["yay", "-Qm"]),
        probe: Some(Probe::Exit(&["pacman", "-Qi"])),
        hint: "https://github.com/Jguer/yay",
        ..BASE
    },
    // ---- Linux: universal ----
    Backend {
        id: "flatpak",
        name: "Flatpak",
        bin: "flatpak",
        kind: Kind::Universal,
        os: &[Os::Linux],
        privilege: Priv::Flexible,
        yes: &["-y"],
        user: &["--user"],
        install: Some(&["flatpak", "install", "{yes}", "{user}"]),
        remove: Some(&["flatpak", "uninstall", "{yes}", "{user}"]),
        purge: Some(&["flatpak", "uninstall", "{yes}", "{user}", "--delete-data"]),
        search: Some(&["flatpak", "search"]),
        info: Some(&["flatpak", "info"]),
        refresh: Some(&["flatpak", "update", "--appstream", "{user}"]),
        upgrade: Some(&["flatpak", "update", "{yes}", "{user}"]),
        list: Some(&["flatpak", "list"]),
        probe: Some(Probe::Exit(&["flatpak", "info"])),
        hint: "https://flatpak.org/setup",
        ..BASE
    },
    Backend {
        id: "snap",
        name: "Snap",
        bin: "snap",
        kind: Kind::Universal,
        os: &[Os::Linux],
        install: Some(&["snap", "install"]),
        remove: Some(&["snap", "remove"]),
        purge: Some(&["snap", "remove", "--purge"]),
        search: Some(&["snap", "find"]),
        info: Some(&["snap", "info"]),
        upgrade: Some(&["snap", "refresh"]),
        list: Some(&["snap", "list"]),
        probe: Some(Probe::Exit(&["snap", "list"])),
        hint: "https://snapcraft.io/docs/installing-snapd",
        ..BASE
    },
    Backend {
        id: "nix",
        name: "Nix",
        bin: "nix",
        kind: Kind::Universal,
        os: &[Os::Linux, Os::Darwin],
        privilege: Priv::User,
        install_prefix: "nixpkgs#",
        install: Some(&["nix", "--extra-experimental-features", "nix-command flakes", "profile", "install"]),
        remove: Some(&["nix", "--extra-experimental-features", "nix-command flakes", "profile", "remove"]),
        search: Some(&["nix", "--extra-experimental-features", "nix-command flakes", "search", "nixpkgs"]),
        upgrade: Some(&["nix", "--extra-experimental-features", "nix-command flakes", "profile", "upgrade", "--all"]),
        list: Some(&["nix", "--extra-experimental-features", "nix-command flakes", "profile", "list"]),
        probe: Some(Probe::Listing(&["nix", "--extra-experimental-features", "nix-command flakes", "profile", "list"])),
        hint: "https://nixos.org/download",
        ..BASE
    },
    BREW,
    // ---- macOS ----
    Backend {
        kind: Kind::Native,
        os: &[Os::Darwin],
        ..BREW
    },
    Backend {
        id: "port",
        name: "MacPorts",
        bin: "port",
        kind: Kind::Community,
        os: &[Os::Darwin],
        yes: &["-N"],
        refresh_before_upgrade: true,
        install: Some(&["port", "{yes}", "install"]),
        remove: Some(&["port", "{yes}", "uninstall"]),
        search: Some(&["port", "search"]),
        info: Some(&["port", "info"]),
        refresh: Some(&["port", "selfupdate"]),
        upgrade: Some(&["port", "{yes}", "upgrade", "outdated"]),
        list: Some(&["port", "installed"]),
        probe: Some(Probe::Marker(&["port", "installed"], "(active)")),
        hint: "https://www.macports.org/install.php",
        ..BASE
    },
    // ---- Windows ----
    Backend {
        id: "winget",
        name: "winget",
        bin: "winget",
        os: &[Os::Windows],
        privilege: Priv::User,
        yes: &["--accept-package-agreements", "--accept-source-agreements"],
        install: Some(&["winget", "install", "{yes}"]),
        remove: Some(&["winget", "uninstall"]),
        purge: Some(&["winget", "uninstall", "--purge"]),
        search: Some(&["winget", "search"]),
        info: Some(&["winget", "show"]),
        refresh: Some(&["winget", "source", "update"]),
        upgrade: Some(&["winget", "upgrade", "--all", "{yes}"]),
        list: Some(&["winget", "list"]),
        probe: Some(Probe::Exit(&["winget", "list"])),
        hint: "https://aka.ms/getwinget",
        ..BASE
    },
    Backend {
        id: "scoop",
        name: "Scoop",
        bin: "scoop",
        os: &[Os::Windows],
        privilege: Priv::User,
        refuses_root: true,
        refresh_before_upgrade: true,
        install: Some(&["scoop", "install"]),
        remove: Some(&["scoop", "uninstall"]),
        purge: Some(&["scoop", "uninstall", "--purge"]),
        search: Some(&["scoop", "search"]),
        info: Some(&["scoop", "info"]),
        refresh: Some(&["scoop", "update"]),
        upgrade: Some(&["scoop", "update", "*"]),
        list: Some(&["scoop", "list"]),
        probe: Some(Probe::Listing(&["scoop", "list"])),
        hint: "https://scoop.sh",
        ..BASE
    },
    Backend {
        id: "choco",
        name: "Chocolatey",
        bin: "choco",
        os: &[Os::Windows],
        yes: &["-y"],
        install: Some(&["choco", "install", "{yes}"]),
        remove: Some(&["choco", "uninstall", "{yes}"]),
        search: Some(&["choco", "search"]),
        info: Some(&["choco", "info"]),
        upgrade: Some(&["choco", "upgrade", "all", "{yes}"]),
        list: Some(&["choco", "list", "--local-only"]),
        probe: Some(Probe::Listing(&["choco", "list", "--local-only"])),
        hint: "https://chocolatey.org/install",
        ..BASE
    },
    // ---- Termux (Android) ----
    Backend {
        id: "pkg",
        name: "Termux pkg",
        bin: "pkg",
        os: &[Os::Android],
        privilege: Priv::User,
        yes: &["-y"],
        install: Some(&["pkg", "install", "{yes}"]),
        remove: Some(&["pkg", "uninstall", "{yes}"]),
        search: Some(&["pkg", "search"]),
        info: Some(&["pkg", "show"]),
        refresh: Some(&["pkg", "update"]),
        upgrade: Some(&["pkg", "upgrade", "{yes}"]),
        list: Some(&["pkg", "list-installed"]),
        probe: Some(DPKG_INSTALLED),
        ..BASE
    },
    // Shares its database with `pkg`, so no refresh/upgrade here.
    Backend {
        id: "apt",
        name: "APT (Termux)",
        bin: "apt",
        os: &[Os::Android],
        privilege: Priv::User,
        yes: &["-y"],
        install: Some(&["apt", "install", "{yes}"]),
        remove: Some(&["apt", "remove", "{yes}"]),
        purge: Some(&["apt", "purge", "{yes}"]),
        search: Some(&["apt", "search"]),
        info: Some(&["apt", "show"]),
        list: Some(&["apt", "list", "--installed"]),
        probe: Some(DPKG_INSTALLED),
        ..BASE
    },
    Backend {
        id: "dpkg",
        name: "dpkg",
        bin: "dpkg",
        os: &[Os::Android],
        privilege: Priv::User,
        local_ext: Some("deb"),
        install: Some(&["dpkg", "-i"]),
        remove: Some(&["dpkg", "-r"]),
        purge: Some(&["dpkg", "-P"]),
        info: Some(&["dpkg", "-s"]),
        list: Some(&["dpkg", "-l"]),
        probe: Some(DPKG_INSTALLED),
        ..BASE
    },
    // ---- BSD ----
    Backend {
        id: "pkg",
        name: "pkg",
        bin: "pkg",
        os: &[Os::FreeBsd, Os::DragonFly],
        yes: &["-y"],
        install: Some(&["pkg", "install", "{yes}"]),
        remove: Some(&["pkg", "delete", "{yes}"]),
        search: Some(&["pkg", "search"]),
        info: Some(&["pkg", "search", "-f"]),
        refresh: Some(&["pkg", "update"]),
        upgrade: Some(&["pkg", "upgrade", "{yes}"]),
        list: Some(&["pkg", "info"]),
        probe: Some(Probe::Exit(&["pkg", "info", "-e"])),
        ..BASE
    },
    Backend {
        id: "pkgin",
        name: "pkgin",
        bin: "pkgin",
        os: &[Os::NetBsd],
        yes: &["-y"],
        refresh_before_upgrade: true,
        install: Some(&["pkgin", "{yes}", "install"]),
        remove: Some(&["pkgin", "{yes}", "remove"]),
        search: Some(&["pkgin", "search"]),
        info: Some(&["pkgin", "pkg-descr"]),
        refresh: Some(&["pkgin", "update"]),
        upgrade: Some(&["pkgin", "{yes}", "full-upgrade"]),
        list: Some(&["pkgin", "list"]),
        probe: Some(Probe::Exit(&["pkg_info", "-e"])),
        ..BASE
    },
    Backend {
        id: "pkg_add",
        name: "pkg_add",
        bin: "pkg_add",
        os: &[Os::OpenBsd],
        install: Some(&["pkg_add"]),
        remove: Some(&["pkg_delete"]),
        search: Some(&["pkg_info", "-Q"]),
        info: Some(&["pkg_info"]),
        upgrade: Some(&["pkg_add", "-u"]),
        list: Some(&["pkg_info"]),
        ..BASE
    },
];

/// Backends that make sense on `p`, in default priority order.
pub fn applicable(p: &Platform) -> Vec<&'static Backend> {
    let mut v: Vec<&Backend> = ALL.iter().filter(|b| p.matches(b.os, b.distros)).collect();
    v.sort_by_key(|b| b.kind);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_unique_per_platform() {
        let distros = ["debian", "arch", "fedora", "suse", "alpine", "void", "gentoo", "solus"];
        let oses = [Os::Linux, Os::Android, Os::Darwin, Os::Windows, Os::FreeBsd, Os::NetBsd, Os::OpenBsd, Os::DragonFly];
        for os in oses {
            for d in distros {
                let p = Platform { os, families: vec![d.into()], pretty: String::new() };
                let ids: Vec<_> = applicable(&p).iter().map(|b| b.id).collect();
                let mut dedup = ids.clone();
                dedup.sort();
                dedup.dedup();
                assert_eq!(ids.len(), dedup.len(), "{os:?}/{d}: {ids:?}");
            }
        }
    }

    #[test]
    fn table_is_well_formed() {
        for b in ALL {
            assert!(!b.id.is_empty() && !b.bin.is_empty() && !b.os.is_empty(), "{}", b.id);
            for op in [Op::Install, Op::Remove, Op::Purge, Op::Search, Op::Info, Op::Refresh, Op::Upgrade, Op::List] {
                if let Some(c) = b.cmd(op) {
                    assert!(!c.is_empty() && !c[0].starts_with('{'), "{} {op:?}", b.id);
                    if c.contains(&"{yes}") {
                        assert!(!b.yes.is_empty(), "{} {op:?} uses {{yes}} without flags", b.id);
                    }
                    if c.contains(&"{user}") {
                        assert!(!b.user.is_empty(), "{} {op:?} uses {{user}} without flags", b.id);
                    }
                }
            }
            assert!(b.install.is_some() && b.remove.is_some(), "{} can't install/remove", b.id);
        }
    }

    #[test]
    fn purge_falls_back_to_remove() {
        let find = |id: &str| ALL.iter().find(|b| b.id == id && b.os.contains(&Os::Linux)).unwrap();
        assert_eq!(find("apt").cmd(Op::Purge).unwrap()[1], "purge");
        assert_eq!(find("dnf").cmd(Op::Purge), find("dnf").cmd(Op::Remove));
        assert_eq!(find("pacman").cmd(Op::Remove).unwrap()[1], "-Rs");
        assert_eq!(find("pacman").cmd(Op::Purge).unwrap()[1], "-Rns");
    }

    #[test]
    fn default_orders() {
        let p = |os, d: &str| Platform { os, families: vec![d.into()], pretty: String::new() };
        let ids = |p: &Platform| applicable(p).iter().map(|b| b.id).collect::<Vec<_>>();
        assert_eq!(ids(&p(Os::Linux, "arch")), ["pacman", "paru", "yay", "flatpak", "snap", "nix", "brew"]);
        assert_eq!(ids(&p(Os::Linux, "debian"))[..2], ["apt", "pacstall"]);
        assert_eq!(ids(&p(Os::Darwin, "")), ["brew", "port", "nix"]);
        assert_eq!(ids(&p(Os::Windows, "")), ["winget", "scoop", "choco"]);
        assert_eq!(ids(&p(Os::Android, "")), ["pkg", "apt", "dpkg"]);
    }
}
