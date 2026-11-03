//! Release builds for a target's CPU, plus the hardening check.
//!
//! The CPU comes from the target itself (`uname -m` over SSH, or the
//! Docker daemon's architecture). A build on the same triple is a plain
//! `cargo build`; anything else cross-builds with `cargo zigbuild`.
//! sccache wraps rustc whenever it is installed. C dependencies (aws-lc,
//! bundled SQLite) get stack protection and fortify flags, Rust links
//! full RELRO, and every shipped ELF must pass [`harden_check`].

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    pub fn triple(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64-unknown-linux-gnu",
            Arch::Aarch64 => "aarch64-unknown-linux-gnu",
        }
    }

    pub fn docker_platform(self) -> &'static str {
        match self {
            Arch::X86_64 => "linux/amd64",
            Arch::Aarch64 => "linux/arm64",
        }
    }

    /// From `uname -m` or Docker's `Architecture` (`x86_64`, `amd64`,
    /// `aarch64`, `arm64`).
    pub fn parse(text: &str) -> Option<Arch> {
        match text.trim() {
            "x86_64" | "amd64" | "linux/amd64" => Some(Arch::X86_64),
            "aarch64" | "arm64" | "linux/arm64" => Some(Arch::Aarch64),
            _ => None,
        }
    }
}

/// This machine's rustc host triple.
pub fn host_triple() -> Result<String, String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .map_err(|e| format!("rustc: {e}"))?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.strip_prefix("host: ").map(str::to_string))
        .ok_or_else(|| "rustc -vV printed no host".to_string())
}

/// CPU of a host reached over SSH.
pub fn detect_ssh(alias: &str, port: Option<u16>) -> Result<Arch, String> {
    let mut cmd = Command::new("ssh");
    cmd.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]);
    if let Some(port) = port {
        cmd.args(["-p", &port.to_string()]);
    }
    let out = cmd
        .arg(alias)
        .arg("uname -m")
        .output()
        .map_err(|e| format!("ssh {alias}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "ssh {alias}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Arch::parse(&text).ok_or_else(|| format!("{alias}: unsupported CPU {:?}", text.trim()))
}

/// CPU of the local Docker daemon.
pub fn detect_docker() -> Result<Arch, String> {
    let out = Command::new("docker")
        .args(["info", "--format", "{{.Architecture}}"])
        .output()
        .map_err(|e| format!("docker: {e}"))?;
    if !out.status.success() {
        return Err("the Docker daemon is not running (start Docker Desktop)".into());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Arch::parse(&text).ok_or_else(|| format!("docker: unsupported CPU {:?}", text.trim()))
}

/// How one build runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildPlan {
    pub arch: Arch,
    /// Cross-build with `cargo zigbuild` (target triple differs from the host's).
    pub zig: bool,
    /// Wrap rustc with sccache.
    pub sccache: bool,
}

pub fn plan(arch: Arch, host: &str, sccache: bool) -> BuildPlan {
    BuildPlan {
        arch,
        zig: host != arch.triple(),
        sccache,
    }
}

pub fn have(bin: &str) -> bool {
    Command::new(bin)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Hardening flags for C code built by `cc` for `triple`.
pub fn hardening_cflags() -> &'static str {
    "-O2 -fstack-protector-strong -fstack-clash-protection -D_FORTIFY_SOURCE=2"
}

/// The environment a release build runs with.
pub fn build_env(plan: &BuildPlan, target_dir: &Path) -> Vec<(String, String)> {
    let triple = plan.arch.triple();
    let mut env = vec![
        (
            "CARGO_TARGET_DIR".to_string(),
            target_dir.display().to_string(),
        ),
        (
            format!("CFLAGS_{}", triple.replace('-', "_")),
            hardening_cflags().to_string(),
        ),
        (
            format!(
                "CARGO_TARGET_{}_RUSTFLAGS",
                triple.replace('-', "_").to_ascii_uppercase()
            ),
            "-C relro-level=full".to_string(),
        ),
    ];
    if plan.sccache {
        env.push(("RUSTC_WRAPPER".to_string(), "sccache".to_string()));
    }
    env
}

/// Build `web` and `xtask` for the plan's triple in `worktree`;
/// returns the directory holding both binaries.
pub fn cargo_release(
    plan: &BuildPlan,
    worktree: &Path,
    target_dir: &Path,
) -> Result<PathBuf, String> {
    let triple = plan.arch.triple();
    if plan.zig && !have("cargo-zigbuild") {
        return Err(
            "cross-building needs cargo-zigbuild (cargo install cargo-zigbuild) and zig".into(),
        );
    }
    let sub = if plan.zig { "zigbuild" } else { "build" };
    let mut cmd = Command::new("cargo");
    cmd.args([
        sub,
        "--release",
        "-p",
        "web",
        "-p",
        "xtask",
        "--target",
        triple,
    ])
    .current_dir(worktree);
    for (k, v) in build_env(plan, target_dir) {
        cmd.env(k, v);
    }
    let status = cmd.status().map_err(|e| format!("cargo {sub}: {e}"))?;
    if !status.success() {
        return Err(format!("cargo {sub} --target {triple} failed"));
    }
    let dir = target_dir.join(triple).join("release");
    for bin in ["web", "xtask"] {
        harden_check(&dir.join(bin)).map_err(|e| format!("{bin}: {e}"))?;
    }
    Ok(dir)
}

// ------------------------------------------------------- hardening check

const PT_LOAD: u32 = 1;
const PT_DYNAMIC: u32 = 2;
const PT_GNU_STACK: u32 = 0x6474_e551;
const PT_GNU_RELRO: u32 = 0x6474_e552;
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const DT_FLAGS: u64 = 30;
const DT_BIND_NOW: u64 = 24;
const DT_FLAGS_1: u64 = 0x6fff_fffb;
const DF_BIND_NOW: u64 = 0x8;
const DF_1_NOW: u64 = 0x1;

/// What a hardened ELF shows.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Hardening {
    pub pie: bool,
    pub nx_stack: bool,
    pub no_wx_segment: bool,
    pub relro: bool,
    pub bind_now: bool,
    pub stack_protector: bool,
}

impl Hardening {
    pub fn problems(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if !self.pie {
            out.push("not position-independent (PIE)");
        }
        if !self.nx_stack {
            out.push("executable stack");
        }
        if !self.no_wx_segment {
            out.push("a segment is both writable and executable");
        }
        if !self.relro {
            out.push("no RELRO segment");
        }
        if !self.bind_now {
            out.push("lazy binding (no BIND_NOW: partial RELRO only)");
        }
        if !self.stack_protector {
            out.push("no stack protector (__stack_chk_fail not imported)");
        }
        out
    }
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}
fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(o..o + 8)?.try_into().ok()?))
}

/// Read the hardening properties of a little-endian ELF64 image.
pub fn inspect(bytes: &[u8]) -> Result<Hardening, String> {
    if bytes.get(0..4) != Some(b"\x7fELF") {
        return Err("not an ELF file".into());
    }
    if bytes.get(4) != Some(&2) || bytes.get(5) != Some(&1) {
        return Err("only little-endian ELF64 is supported".into());
    }
    let bad = || "truncated ELF header".to_string();
    let e_type = u16_at(bytes, 16).ok_or_else(bad)?;
    let phoff = u64_at(bytes, 32).ok_or_else(bad)? as usize;
    let phentsize = u16_at(bytes, 54).ok_or_else(bad)? as usize;
    let phnum = u16_at(bytes, 56).ok_or_else(bad)? as usize;
    let mut h = Hardening {
        pie: e_type == 3,
        no_wx_segment: true,
        ..Hardening::default()
    };
    let mut saw_stack = false;
    for i in 0..phnum {
        let o = phoff + i * phentsize;
        let p_type = u32_at(bytes, o).ok_or_else(bad)?;
        let p_flags = u32_at(bytes, o + 4).ok_or_else(bad)?;
        let p_offset = u64_at(bytes, o + 8).ok_or_else(bad)? as usize;
        let p_filesz = u64_at(bytes, o + 32).ok_or_else(bad)? as usize;
        match p_type {
            PT_GNU_STACK => {
                saw_stack = true;
                h.nx_stack = p_flags & PF_X == 0;
            }
            PT_GNU_RELRO => h.relro = true,
            PT_LOAD if p_flags & PF_W != 0 && p_flags & PF_X != 0 => h.no_wx_segment = false,
            PT_DYNAMIC => {
                let end = p_offset.saturating_add(p_filesz).min(bytes.len());
                let mut d = p_offset;
                while d + 16 <= end {
                    let tag = u64_at(bytes, d).ok_or_else(bad)?;
                    let val = u64_at(bytes, d + 8).ok_or_else(bad)?;
                    match tag {
                        0 => break,
                        DT_BIND_NOW => h.bind_now = true,
                        DT_FLAGS if val & DF_BIND_NOW != 0 => h.bind_now = true,
                        DT_FLAGS_1 if val & DF_1_NOW != 0 => h.bind_now = true,
                        _ => {}
                    }
                    d += 16;
                }
            }
            _ => {}
        }
    }
    // No PT_GNU_STACK at all means the kernel default (executable).
    h.nx_stack &= saw_stack;
    h.stack_protector = bytes.windows(17).any(|w| w == b"__stack_chk_fail\0");
    Ok(h)
}

/// Fail unless the ELF at `path` is hardened.
pub fn harden_check(path: &Path) -> Result<Hardening, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let h = inspect(&bytes)?;
    let problems = h.problems();
    if problems.is_empty() {
        Ok(h)
    } else {
        Err(format!(
            "{} is not hardened: {}",
            path.display(),
            problems.join("; ")
        ))
    }
}

/// `cargo xtask harden-check <elf>...`
pub fn run_harden_check(args: &[String]) {
    if args.is_empty() {
        eprintln!("usage: cargo xtask harden-check <elf> [<elf>...]");
        std::process::exit(2);
    }
    let mut failed = false;
    for path in args {
        match harden_check(Path::new(path)) {
            Ok(_) => {
                println!("hardened: {path} (PIE, NX stack, no W+X, full RELRO, stack protector)")
            }
            Err(e) => {
                eprintln!("{e}");
                failed = true;
            }
        }
    }
    if failed {
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal ELF64 image with the given program headers and
    /// dynamic entries (enough for [`inspect`]).
    fn elf(e_type: u16, phdrs: &[(u32, u32)], dynamic: &[(u64, u64)], canary: bool) -> Vec<u8> {
        let mut b = vec![0u8; 64];
        b[0..4].copy_from_slice(b"\x7fELF");
        b[4] = 2;
        b[5] = 1;
        b[16..18].copy_from_slice(&e_type.to_le_bytes());
        let phoff = 64usize;
        b[32..40].copy_from_slice(&(phoff as u64).to_le_bytes());
        b[54..56].copy_from_slice(&56u16.to_le_bytes());
        let count = phdrs.len() + usize::from(!dynamic.is_empty());
        b[56..58].copy_from_slice(&(count as u16).to_le_bytes());
        let dyn_off = phoff + count * 56;
        let mut ph = Vec::new();
        for (t, f) in phdrs {
            let mut p = vec![0u8; 56];
            p[0..4].copy_from_slice(&t.to_le_bytes());
            p[4..8].copy_from_slice(&f.to_le_bytes());
            ph.extend(p);
        }
        if !dynamic.is_empty() {
            let mut p = vec![0u8; 56];
            p[0..4].copy_from_slice(&PT_DYNAMIC.to_le_bytes());
            p[8..16].copy_from_slice(&(dyn_off as u64).to_le_bytes());
            p[32..40].copy_from_slice(&((dynamic.len() as u64 + 1) * 16).to_le_bytes());
            ph.extend(p);
        }
        b.extend(ph);
        for (tag, val) in dynamic {
            b.extend(tag.to_le_bytes());
            b.extend(val.to_le_bytes());
        }
        b.extend([0u8; 16]);
        if canary {
            b.extend(b"__stack_chk_fail\0");
        }
        b
    }

    #[test]
    fn a_hardened_image_passes_every_check() {
        let image = elf(
            3,
            &[
                (PT_LOAD, 5),
                (PT_LOAD, 6),
                (PT_GNU_STACK, 6),
                (PT_GNU_RELRO, 4),
            ],
            &[(DT_FLAGS, DF_BIND_NOW), (DT_FLAGS_1, DF_1_NOW)],
            true,
        );
        let h = inspect(&image).unwrap();
        assert!(h.problems().is_empty(), "{:?}", h.problems());
    }

    #[test]
    fn each_missing_protection_is_named() {
        let image = elf(2, &[(PT_LOAD, 7), (PT_GNU_STACK, 7)], &[], false);
        let problems = inspect(&image).unwrap().problems().join("; ");
        for needle in [
            "PIE",
            "executable stack",
            "writable and executable",
            "RELRO",
            "lazy binding",
            "stack protector",
        ] {
            assert!(problems.contains(needle), "{needle}: {problems}");
        }
        // No PT_GNU_STACK at all is an executable stack too.
        let image = elf(3, &[(PT_GNU_RELRO, 4)], &[(DT_BIND_NOW, 0)], true);
        assert!(!inspect(&image).unwrap().nx_stack);
        assert!(inspect(b"not an elf").is_err());
    }

    #[test]
    fn cpus_and_cross_builds_are_detected() {
        assert_eq!(Arch::parse("x86_64\n"), Some(Arch::X86_64));
        assert_eq!(Arch::parse("arm64"), Some(Arch::Aarch64));
        assert_eq!(Arch::parse("riscv64"), None);
        let mac = plan(Arch::Aarch64, "aarch64-apple-darwin", true);
        assert!(mac.zig, "darwin to linux is a cross build");
        let plant = plan(Arch::X86_64, "x86_64-unknown-linux-gnu", false);
        assert!(!plant.zig);
        let env = build_env(&mac, Path::new("/tmp/t"));
        assert!(env.contains(&("RUSTC_WRAPPER".into(), "sccache".into())));
        assert!(
            env.iter()
                .any(|(k, v)| k == "CFLAGS_aarch64_unknown_linux_gnu"
                    && v.contains("-fstack-protector-strong"))
        );
        assert!(env.iter().any(
            |(k, v)| k == "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUSTFLAGS"
                && v.contains("relro-level=full")
        ));
    }
}
