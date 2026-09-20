use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Linux,
    Android,
    Darwin,
    Windows,
    FreeBsd,
    NetBsd,
    OpenBsd,
    DragonFly,
}

impl Os {
    pub fn name(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::Android => "android (termux)",
            Os::Darwin => "macos",
            Os::Windows => "windows",
            Os::FreeBsd => "freebsd",
            Os::NetBsd => "netbsd",
            Os::OpenBsd => "openbsd",
            Os::DragonFly => "dragonfly",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Platform {
    pub os: Os,
    /// os-release `ID` followed by `ID_LIKE`, lowercased. Empty outside Linux.
    pub families: Vec<String>,
    pub pretty: String,
}

impl Platform {
    pub fn detect() -> Result<Self> {
        let os = match env::consts::OS {
            "linux" if is_termux() => Os::Android,
            "linux" => Os::Linux,
            "android" => Os::Android,
            "macos" => Os::Darwin,
            "windows" => Os::Windows,
            "freebsd" => Os::FreeBsd,
            "netbsd" => Os::NetBsd,
            "openbsd" => Os::OpenBsd,
            "dragonfly" => Os::DragonFly,
            other => bail!("unsupported operating system: {other}"),
        };
        let (families, pretty) = if os == Os::Linux {
            fs::read_to_string("/etc/os-release")
                .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
                .map(|t| parse_os_release(&t))
                .unwrap_or_else(|_| (Vec::new(), "Linux".into()))
        } else {
            (Vec::new(), os.name().to_string())
        };
        Ok(Platform { os, families, pretty })
    }

    /// Whether a backend scoped to `os` (and, on Linux, `distros`) belongs on this platform.
    /// An unreadable os-release leaves `families` empty, which lets distro-gated backends through
    /// so that an unusual distro still works if the binary is present.
    pub fn matches(&self, os: &[Os], distros: &[&str]) -> bool {
        os.contains(&self.os)
            && (distros.is_empty()
                || self.families.is_empty()
                || distros.iter().any(|d| self.families.iter().any(|f| f == d)))
    }
}

fn is_termux() -> bool {
    env::var_os("TERMUX_VERSION").is_some()
        || env::var("PREFIX").is_ok_and(|p| p.contains("com.termux"))
}

pub fn parse_os_release(text: &str) -> (Vec<String>, String) {
    let mut id = String::new();
    let mut like = String::new();
    let mut pretty = String::new();
    for line in text.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim().trim_matches(|c| c == '"' || c == '\'');
        match k.trim() {
            "ID" => id = v.to_lowercase(),
            "ID_LIKE" => like = v.to_lowercase(),
            "PRETTY_NAME" => pretty = v.to_string(),
            _ => {}
        }
    }
    let mut families: Vec<String> = Vec::new();
    for f in std::iter::once(id.as_str()).chain(like.split_whitespace()) {
        if !f.is_empty() && !families.iter().any(|x| x == f) {
            families.push(f.to_string());
        }
    }
    if pretty.is_empty() {
        pretty = "Linux".into();
    }
    (families, pretty)
}

/// Whether the current process has root (unix) or administrator (Windows) rights.
pub fn is_root(os: Os) -> bool {
    let quiet = |mut c: Command| {
        c.stdin(Stdio::null()).stderr(Stdio::null());
        c
    };
    if os == Os::Windows {
        // Only elevated processes can read the LocalService hive.
        return quiet(Command::new("reg"))
            .args(["query", r"HKU\S-1-5-19"])
            .stdout(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    }
    quiet(Command::new("id"))
        .arg("-u")
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
}

/// Resolve an executable on `PATH` without shelling out. Honors `PATHEXT` on Windows.
pub fn which(bin: &str) -> Option<PathBuf> {
    let paths = env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) {
        let pathext = env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        std::iter::once(String::new())
            .chain(pathext.split(';').filter(|e| !e.is_empty()).map(str::to_lowercase))
            .collect()
    } else {
        vec![String::new()]
    };
    env::split_paths(&paths).find_map(|dir| {
        exts.iter()
            .map(|ext| dir.join(format!("{bin}{ext}")))
            .find(|p| is_executable(p))
    })
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    // Windows app-execution aliases (e.g. winget.exe) are reparse points that `metadata` rejects.
    fs::symlink_metadata(p).is_ok_and(|m| !m.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_release_families() {
        let (f, pretty) = parse_os_release(
            "NAME=\"Manjaro Linux\"\nID=manjaro\nID_LIKE=arch\nPRETTY_NAME=\"Manjaro Linux\"\n",
        );
        assert_eq!(f, ["manjaro", "arch"]);
        assert_eq!(pretty, "Manjaro Linux");
        let (f, _) = parse_os_release("ID=ubuntu\nID_LIKE=\"debian\"\n");
        assert_eq!(f, ["ubuntu", "debian"]);
        let (f, _) = parse_os_release("ID=debian\n");
        assert_eq!(f, ["debian"]);
    }

    #[test]
    fn distro_gating() {
        let arch = Platform { os: Os::Linux, families: vec!["manjaro".into(), "arch".into()], pretty: String::new() };
        assert!(arch.matches(&[Os::Linux], &["arch"]));
        assert!(!arch.matches(&[Os::Linux], &["debian", "ubuntu"]));
        assert!(arch.matches(&[Os::Linux, Os::Darwin], &[]));
        assert!(!arch.matches(&[Os::Windows], &[]));
    }
}
