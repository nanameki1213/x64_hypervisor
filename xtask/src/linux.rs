//! `cargo xtask linux`: 管理 VM（Linux）のカーネルと initramfs を作り、ESP に置く

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{DynError, project_root};

/// ビルドするカーネル（kernel.org の longterm）。更新するときは SHA-256 も sha256sums.asc から更新する
const LINUX_VERSION: &str = "6.18.62";
const LINUX_SHA256: &str = "490a7aedf1b355c565d3a6c05333ccccb5da82064eca46b590448a7099d7d651";

/// ESP 上のファイル名（bootloader/src/main.rs の LINUX_*_FILE と一致させる）
pub const ESP_KERNEL: &str = "bzImage";
const ESP_INITRD: &str = "initrd.img";
const ESP_CMDLINE: &str = "cmdline.txt";

/// ソースの展開とビルドを行うディレクトリ。
/// ネットワークファイルシステム上だと遅いので、環境変数でローカルディスクに変えられるようにする
fn work_dir() -> PathBuf {
    env::var_os("XTASK_LINUX_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| project_root().join("build/linux"))
}

fn run(cmd: &mut Command) -> Result<(), DynError> {
    let status = cmd.status()?;
    if !status.success() {
        return Err(format!("command failed ({}): {:?}", status, cmd).into());
    }
    Ok(())
}

pub fn build() -> Result<(), DynError> {
    let work = work_dir();
    fs::create_dir_all(&work)?;
    let src = fetch_source(&work)?;
    let obj = work.join("obj");
    let bz_image = build_kernel(&src, &obj)?;

    let esp = project_root().join("esp");
    fs::copy(&bz_image, esp.join(ESP_KERNEL))?;
    eprintln!("Copied {} -> esp/{}", bz_image.display(), ESP_KERNEL);

    let initrd = build_initramfs()?;
    fs::write(esp.join(ESP_INITRD), &initrd)?;
    eprintln!("Wrote esp/{} ({} bytes)", ESP_INITRD, initrd.len());

    fs::copy(
        project_root().join("linux/cmdline.txt"),
        esp.join(ESP_CMDLINE),
    )?;
    eprintln!("Copied linux/cmdline.txt -> esp/{}", ESP_CMDLINE);
    Ok(())
}

/// tarball を取得・検証して展開し、ソースディレクトリを返す
fn fetch_source(work: &Path) -> Result<PathBuf, DynError> {
    let name = format!("linux-{}", LINUX_VERSION);
    let src = work.join(&name);
    if src.join("Makefile").exists() {
        return Ok(src);
    }

    let tarball = work.join(format!("{}.tar.xz", name));
    if !tarball.exists() {
        let major = LINUX_VERSION.split('.').next().unwrap();
        let url = format!(
            "https://cdn.kernel.org/pub/linux/kernel/v{}.x/{}.tar.xz",
            major, name
        );
        eprintln!("Downloading {}", url);
        let partial = tarball.with_extension("xz.part");
        run(Command::new("curl")
            .args(["-fL", "--retry", "3", "-o"])
            .arg(&partial)
            .arg(&url))?;
        fs::rename(&partial, &tarball)?;
    }

    let output = Command::new("sha256sum").arg(&tarball).output()?;
    let digest = String::from_utf8_lossy(&output.stdout);
    let digest = digest.split_whitespace().next().unwrap_or("");
    if digest != LINUX_SHA256 {
        return Err(format!(
            "SHA-256 mismatch for {}: expected {}, got {}. Delete the file and retry.",
            tarball.display(),
            LINUX_SHA256,
            digest
        )
        .into());
    }

    // 展開途中で止まっても壊れたソースを使わないよう、別の名前で展開してから置き換える
    let staging = work.join(format!("{}.extract", name));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging)?;
    eprintln!("Extracting {}", tarball.display());
    let status = Command::new("tar")
        .args(["xf"])
        .arg(&tarball)
        .args(["--no-same-owner", "--no-same-permissions", "-C"])
        .arg(&staging)
        .status()?;
    let extracted = staging.join(&name);
    // ファイルシステムによってはシンボリックリンクの chmod が失敗して tar が 2 を返すが、
    // 中身は展開できているので、Makefile があれば続ける
    if !extracted.join("arch/x86/Makefile").exists() {
        return Err(format!("failed to extract {} ({})", tarball.display(), status).into());
    }
    if !status.success() {
        eprintln!(
            "warning: tar exited with {} (ignored: the source tree is complete)",
            status
        );
    }
    fs::rename(&extracted, &src)?;
    let _ = fs::remove_dir_all(&staging);
    Ok(src)
}

/// `allnoconfig` に linux/mgmt-vm.config を重ねて設定し、bzImage をビルドする
fn build_kernel(src: &Path, obj: &Path) -> Result<PathBuf, DynError> {
    let fragment = project_root().join("linux/mgmt-vm.config");
    let make = |target: &str| {
        let mut cmd = Command::new("make");
        cmd.current_dir(src)
            .arg(format!("O={}", obj.display()))
            .arg("ARCH=x86_64")
            .arg(target);
        cmd
    };

    run(make("allnoconfig").arg(format!("KCONFIG_ALLCONFIG={}", fragment.display())))?;
    check_config(&fragment, &obj.join(".config"))?;

    let jobs = std::thread::available_parallelism().map_or(1, |n| n.get());
    run(make("bzImage").arg(format!("-j{}", jobs)))?;
    Ok(obj.join("arch/x86/boot/bzImage"))
}

/// 依存関係が満たされずに無視された設定がないか確かめる
fn check_config(fragment: &Path, config: &Path) -> Result<(), DynError> {
    let wanted = fs::read_to_string(fragment)?;
    let actual = fs::read_to_string(config)?;
    let lines: Vec<&str> = actual.lines().collect();
    let mut missing = Vec::new();
    for line in wanted.lines().map(str::trim) {
        let ok = if line.starts_with("CONFIG_") {
            lines.contains(&line)
        } else if let Some(name) = line
            .strip_prefix("# ")
            .and_then(|l| l.strip_suffix(" is not set"))
        {
            !lines.iter().any(|l| l.starts_with(&format!("{}=", name)))
        } else {
            true
        };
        if !ok {
            missing.push(line);
        }
    }
    if !missing.is_empty() {
        return Err(format!(
            "these options in {} were not applied (missing dependencies?):\n  {}",
            fragment.display(),
            missing.join("\n  ")
        )
        .into());
    }
    Ok(())
}

/// busybox（静的リンク）と linux/init から、無圧縮の initramfs（cpio newc）を作る
fn build_initramfs() -> Result<Vec<u8>, DynError> {
    let busybox_path = find_busybox()?;
    let busybox = fs::read(&busybox_path)?;
    if is_dynamically_linked(&busybox) {
        return Err(format!(
            "{} is dynamically linked. Install a static busybox (e.g. busybox-static) or set BUSYBOX.",
            busybox_path.display()
        )
        .into());
    }
    let init = fs::read(project_root().join("linux/init"))?;

    let mut cpio = Cpio::default();
    for dir in ["bin", "dev", "proc", "sys", "tmp"] {
        cpio.dir(dir);
    }
    // devtmpfs をマウントする前に、カーネルが /init の標準入出力として開く
    cpio.char_device("dev/console", 0o600, 5, 1);
    cpio.file("bin/busybox", 0o755, &busybox);
    cpio.symlink("bin/sh", "busybox");
    cpio.file("init", 0o755, &init);
    eprintln!("initramfs: busybox from {}", busybox_path.display());
    Ok(cpio.finish())
}

fn find_busybox() -> Result<PathBuf, DynError> {
    if let Some(path) = env::var_os("BUSYBOX") {
        return Ok(PathBuf::from(path));
    }
    env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join("busybox"))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            "busybox not found. Install a static busybox (e.g. busybox-static) or set BUSYBOX."
                .into()
        })
}

/// ELF64 のプログラムヘッダに PT_INTERP があるか
fn is_dynamically_linked(elf: &[u8]) -> bool {
    const PT_INTERP: u32 = 3;
    let u16_at = |o: usize| u16::from_le_bytes([elf[o], elf[o + 1]]) as usize;
    if elf.len() < 64 || &elf[..4] != b"\x7fELF" {
        return false;
    }
    let phoff = u64::from_le_bytes(elf[0x20..0x28].try_into().unwrap()) as usize;
    let (phentsize, phnum) = (u16_at(0x36), u16_at(0x38));
    (0..phnum).any(|i| {
        let o = phoff + i * phentsize;
        elf.get(o..o + 4)
            .is_some_and(|t| u32::from_le_bytes(t.try_into().unwrap()) == PT_INTERP)
    })
}

/// cpio（newc 形式）の書き出し
#[derive(Default)]
struct Cpio {
    out: Vec<u8>,
    ino: u32,
}

impl Cpio {
    fn entry(&mut self, name: &str, mode: u32, rdev: (u32, u32), data: &[u8]) {
        self.ino += 1;
        let nlink = if mode & 0o170000 == 0o040000 { 2 } else { 1 };
        let fields = [
            self.ino,
            mode,
            0, // uid
            0, // gid
            nlink,
            0, // mtime
            data.len() as u32,
            0, // devmajor
            0, // devminor
            rdev.0,
            rdev.1,
            name.len() as u32 + 1,
            0, // check
        ];
        self.out.extend_from_slice(b"070701");
        for f in fields {
            self.out.extend_from_slice(format!("{:08x}", f).as_bytes());
        }
        self.out.extend_from_slice(name.as_bytes());
        self.out.push(0);
        self.pad();
        self.out.extend_from_slice(data);
        self.pad();
    }

    fn pad(&mut self) {
        while !self.out.len().is_multiple_of(4) {
            self.out.push(0);
        }
    }

    fn dir(&mut self, name: &str) {
        self.entry(name, 0o040755, (0, 0), &[]);
    }

    fn file(&mut self, name: &str, perm: u32, data: &[u8]) {
        self.entry(name, 0o100000 | perm, (0, 0), data);
    }

    fn symlink(&mut self, name: &str, target: &str) {
        self.entry(name, 0o120777, (0, 0), target.as_bytes());
    }

    fn char_device(&mut self, name: &str, perm: u32, major: u32, minor: u32) {
        self.entry(name, 0o020000 | perm, (major, minor), &[]);
    }

    fn finish(mut self) -> Vec<u8> {
        self.entry("TRAILER!!!", 0, (0, 0), &[]);
        self.out
    }
}
