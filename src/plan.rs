//! Turns a backend + operation into a concrete argv, deciding on privileges along the way.

use std::path::PathBuf;

use crate::{
    backends::{self, Backend, Op, Priv},
    config::Sources,
    platform::{Platform, which},
};

#[derive(Clone)]
pub struct Detected {
    pub backend: &'static Backend,
    pub path: PathBuf,
}

pub struct Ctx {
    pub is_root: bool,
    /// User-only mode: never use root, skip backends that need it.
    pub user_mode: bool,
    pub escalator: Option<String>,
    pub yes: bool,
}

pub struct Access {
    /// Prefix the command with the escalator.
    pub escalate: bool,
    /// Operating system-wide (root) rather than per-user.
    pub system: bool,
}

impl Ctx {
    pub fn access(&self, b: &Backend, op: Op) -> Result<Access, String> {
        if b.refuses_root && self.is_root {
            return Err("refuses to run as root".into());
        }
        let system = self.is_root && !self.user_mode;
        let mut escalate = false;
        if op.mutating() && b.privilege == Priv::Root && !system {
            if self.user_mode {
                return Err("needs root (user-only mode)".into());
            }
            if self.escalator.is_none() {
                return Err("needs root and no sudo/doas was found".into());
            }
            escalate = true;
        }
        Ok(Access { escalate, system })
    }

    /// Full argv for `op` on `d`. `pkgs` are already the backend's names for the packages.
    pub fn argv(&self, d: &Detected, op: Op, pkgs: &[String], extra: &[String]) -> Result<Vec<String>, String> {
        let b = d.backend;
        let cmd = b.cmd(op).ok_or_else(|| format!("{} does not support this operation", b.id))?;
        let access = self.access(b, op)?;
        let mut out = expand(cmd, d, self.yes, !access.system);
        if op == Op::Install {
            out.extend(extra.iter().cloned());
            out.extend(pkgs.iter().map(|p| format!("{}{p}", b.install_prefix)));
        } else {
            out.extend(pkgs.iter().cloned());
        }
        if access.escalate {
            out.insert(0, self.escalator.clone().unwrap_or_default());
        }
        Ok(out)
    }
}

/// Expand `{yes}`/`{user}` and swap the backend's own executable for its resolved path.
pub fn expand(cmd: &[&str], d: &Detected, yes: bool, user: bool) -> Vec<String> {
    let b = d.backend;
    let mut out = Vec::with_capacity(cmd.len());
    for (i, tok) in cmd.iter().enumerate() {
        match *tok {
            "{yes}" => {
                if yes {
                    out.extend(b.yes.iter().map(|s| s.to_string()));
                }
            }
            "{user}" => {
                if user {
                    out.extend(b.user.iter().map(|s| s.to_string()));
                }
            }
            t if i == 0 && t == b.bin => out.push(d.path.to_string_lossy().into_owned()),
            t => out.push(t.to_string()),
        }
    }
    out
}

/// Backends applicable here and installed, in effective priority order.
pub fn detect(p: &Platform, src: &Sources) -> Vec<Detected> {
    let mut found: Vec<Detected> = backends::applicable(p)
        .into_iter()
        .filter(|b| !src.disabled.iter().any(|d| d == b.id))
        .filter_map(|b| which(b.bin).map(|path| Detected { backend: b, path }))
        .collect();
    // Stable: backends missing from `order` keep their default relative order, after the listed ones.
    found.sort_by_key(|d| src.order.iter().position(|i| i == d.backend.id).unwrap_or(usize::MAX));
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Os;

    fn det(id: &str, os: Os) -> Detected {
        let backend = backends::ALL.iter().find(|b| b.id == id && b.os.contains(&os)).unwrap();
        Detected { backend, path: PathBuf::from(format!("/bin/{}", backend.bin)) }
    }

    fn ctx(is_root: bool, user_mode: bool, sudo: bool, yes: bool) -> Ctx {
        Ctx { is_root, user_mode, escalator: sudo.then(|| "sudo".to_string()), yes }
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn root_runs_directly() {
        let argv = ctx(true, false, false, true).argv(&det("apt", Os::Linux), Op::Install, &s(&["vim"]), &[]).unwrap();
        assert_eq!(argv, ["/bin/apt-get", "install", "-y", "vim"]);
    }

    #[test]
    fn user_escalates_for_root_backends() {
        let argv = ctx(false, false, true, false).argv(&det("pacman", Os::Linux), Op::Install, &s(&["vim"]), &[]).unwrap();
        assert_eq!(argv, ["sudo", "/bin/pacman", "-S", "--needed", "vim"]);
        // Read-only operations never escalate.
        let argv = ctx(false, false, true, false).argv(&det("pacman", Os::Linux), Op::Search, &s(&["vim"]), &[]).unwrap();
        assert_eq!(argv, ["/bin/pacman", "-Ss", "vim"]);
    }

    #[test]
    fn user_mode_skips_root_backends_but_not_user_ones() {
        let c = ctx(false, true, true, false);
        assert!(c.argv(&det("pacman", Os::Linux), Op::Install, &s(&["vim"]), &[]).is_err());
        assert!(c.argv(&det("pacman", Os::Linux), Op::Search, &s(&["vim"]), &[]).is_ok());
        assert!(c.argv(&det("paru", Os::Linux), Op::Install, &s(&["vim"]), &[]).is_ok());
        let c = ctx(false, false, false, false);
        assert!(c.argv(&det("apt", Os::Linux), Op::Install, &s(&["vim"]), &[]).is_err());
    }

    #[test]
    fn flatpak_scope_follows_privileges() {
        let f = det("flatpak", Os::Linux);
        let user = ctx(false, false, true, true).argv(&f, Op::Install, &s(&["org.x.App"]), &[]).unwrap();
        assert_eq!(user, ["/bin/flatpak", "install", "-y", "--user", "org.x.App"]);
        let root = ctx(true, false, false, true).argv(&f, Op::Install, &s(&["org.x.App"]), &[]).unwrap();
        assert_eq!(root, ["/bin/flatpak", "install", "-y", "org.x.App"]);
    }

    #[test]
    fn root_refusers_are_skipped_as_root() {
        assert!(ctx(true, false, false, false).argv(&det("paru", Os::Linux), Op::Install, &s(&["x"]), &[]).is_err());
        assert!(ctx(true, false, false, false).argv(&det("brew", Os::Darwin), Op::Search, &s(&["x"]), &[]).is_err());
    }

    #[test]
    fn install_prefix_and_extra_args() {
        let argv = ctx(false, false, false, false)
            .argv(&det("nix", Os::Linux), Op::Install, &s(&["hello"]), &s(&["--impure"]))
            .unwrap();
        assert_eq!(argv.last().unwrap(), "nixpkgs#hello");
        assert_eq!(argv[argv.len() - 2], "--impure");
    }
}
