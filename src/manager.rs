//! Installation and selection of per-user debugger versions.
use std::{
    env,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

/// Most recent installation stage and, when available, actual download measurements.
#[derive(Clone, Debug, Default)]
pub struct InstallProgress {
    pub stage: String,
    pub percent: Option<f32>,
    pub downloaded: Option<u64>,
    pub total: Option<u64>,
    pub bytes_per_second: Option<u64>,
    pub remaining_seconds: Option<u64>,
}
pub type SharedInstallProgress = Arc<Mutex<InstallProgress>>;

fn stage(progress: &SharedInstallProgress, message: &str) {
    if let Ok(mut value) = progress.lock() {
        *value = InstallProgress {
            stage: message.into(),
            ..Default::default()
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    OpenOcd,
    PyOcd,
    ProbeRs,
}
impl Tool {
    pub const ALL: [Self; 3] = [Self::OpenOcd, Self::PyOcd, Self::ProbeRs];
    fn executable(self) -> &'static str {
        match self {
            Self::OpenOcd => "openocd",
            Self::PyOcd => "pyocd",
            Self::ProbeRs => "probe-rs",
        }
    }
}
impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.executable())
    }
}
impl FromStr for Tool {
    type Err = String;
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "openocd" => Ok(Self::OpenOcd),
            "pyocd" => Ok(Self::PyOcd),
            "probe-rs" | "probers" => Ok(Self::ProbeRs),
            _ => Err(format!("unknown debugger: {s}")),
        }
    }
}

pub struct Manager {
    pub config_path: PathBuf,
    pub data_dir: PathBuf,
}
impl Manager {
    fn local_path(&self) -> PathBuf {
        self.config_path.with_extension("local.json")
    }
    fn local_entries(&self) -> Result<serde_json::Map<String, serde_json::Value>> {
        match fs::read(self.local_path()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
            Err(e) => Err(e.into()),
        }
    }
    fn save_local_entries(
        &self,
        entries: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<()> {
        fs::create_dir_all(self.config_path.parent().ok_or("invalid config path")?)?;
        fs::write(self.local_path(), serde_json::to_vec_pretty(entries)?)?;
        Ok(())
    }
    /// Register an existing debugger directory without copying or modifying its files.
    pub fn add_local(&self, tool: Tool, name: &str, directory: &Path) -> Result<()> {
        validate_local_name(name)?;
        if !directory.is_dir() {
            return Err(format!("not a directory: {}", directory.display()).into());
        }
        let directory = fs::canonicalize(directory)?;
        let names = if cfg!(windows) {
            vec![
                format!("{}.exe", tool.executable()),
                format!("{}.cmd", tool.executable()),
                format!("{}.bat", tool.executable()),
            ]
        } else {
            vec![tool.executable().to_owned()]
        };
        let executable = [
            directory.clone(),
            directory.join("bin"),
            directory.join("Scripts"),
        ]
        .iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "{tool} executable not found in {} or its bin/Scripts directory",
                directory.display()
            )
        })?;
        let executable = executable.to_str().ok_or("local path is not valid UTF-8")?;
        let key = format!("{tool}:local:{name}");
        let mut entries = self.local_entries()?;
        if entries.contains_key(&key) {
            return Err(format!("local {tool} {name} already exists").into());
        }
        entries.insert(key, executable.into());
        self.save_local_entries(&entries)
    }
    pub fn local(&self, tool: Tool) -> Result<Vec<(String, PathBuf)>> {
        let prefix = format!("{tool}:local:");
        let mut found = Vec::new();
        for (key, value) in self.local_entries()? {
            if let Some(name) = key.strip_prefix(&prefix) {
                let path = value
                    .as_str()
                    .ok_or("invalid local debugger path in config")?;
                found.push((name.to_owned(), PathBuf::from(path)));
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(found)
    }
    /// Remove a registration, leaving the original files untouched.
    pub fn remove_local(&self, tool: Tool, name: &str) -> Result<()> {
        validate_local_name(name)?;
        let mut entries = self.local_entries()?;
        if entries.remove(&format!("{tool}:local:{name}")).is_none() {
            return Err(format!("local {tool} {name} is not registered").into());
        }
        let selection = format!("local:{name}");
        if self.selected(tool)?.as_deref() == Some(&selection) {
            let content = fs::read_to_string(&self.config_path)?;
            let key = format!("{tool}=");
            let remaining = content
                .lines()
                .filter(|line| !line.starts_with(&key))
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(
                &self.config_path,
                if remaining.is_empty() {
                    String::new()
                } else {
                    format!("{remaining}\n")
                },
            )?;
        }
        self.save_local_entries(&entries)
    }
    pub fn new() -> Result<Self> {
        let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .ok_or("cannot determine user home directory")?;
        let home = PathBuf::from(home);
        let (config, data) = if cfg!(windows) {
            (
                env::var_os("APPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData/Roaming")),
                env::var_os("LOCALAPPDATA")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join("AppData/Local")),
            )
        } else if cfg!(target_os = "macos") {
            (
                home.join("Library/Application Support"),
                home.join("Library/Application Support"),
            )
        } else {
            (
                env::var_os("XDG_CONFIG_HOME")
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".config")),
                env::var_os("XDG_DATA_HOME")
                    .filter(|s| !s.is_empty())
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".local/share")),
            )
        };
        Ok(Self {
            config_path: config.join("NyaScrapingCore/config"),
            data_dir: data.join("NyaScrapingCore/debuggers"),
        })
    }
    fn executable(&self, tool: Tool, version: &str) -> PathBuf {
        let root = self.data_dir.join(tool.to_string()).join(version);
        let name = if cfg!(windows) {
            format!("{}.exe", tool.executable())
        } else {
            tool.executable().to_owned()
        };
        match tool {
            Tool::PyOcd if cfg!(windows) => root.join("Scripts").join(name),
            Tool::OpenOcd => root
                .join(format!("xpack-openocd-{version}"))
                .join("bin")
                .join(name),
            _ => root.join("bin").join(name),
        }
    }
    pub fn installed(&self, tool: Tool) -> Result<Vec<(String, PathBuf)>> {
        let root = self.data_dir.join(tool.to_string());
        if !root.exists() {
            return Ok(vec![]);
        }
        let mut found = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if let Some(version) = entry.file_name().to_str() {
                let path = self.executable(tool, version);
                if path.is_file() {
                    found.push((version.to_owned(), path));
                }
            }
        }
        found.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(found)
    }
    pub fn system(&self, tool: Tool) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(path) = env::var_os("PATH") {
            for dir in env::split_paths(&path) {
                let names = if cfg!(windows) {
                    vec![
                        format!("{}.exe", tool.executable()),
                        format!("{}.cmd", tool.executable()),
                        format!("{}.bat", tool.executable()),
                    ]
                } else {
                    vec![tool.executable().to_owned()]
                };
                for name in names {
                    let path = dir.join(name);
                    if path.is_file() && !paths.contains(&path) {
                        paths.push(path);
                    }
                }
            }
        }
        paths
    }
    pub fn version(&self, path: &Path) -> String {
        match Command::new(path).arg("--version").output() {
            Ok(out) => {
                let text = if out.stdout.is_empty() {
                    &out.stderr
                } else {
                    &out.stdout
                };
                let line = String::from_utf8_lossy(text)
                    .lines()
                    .next()
                    .unwrap_or("unknown")
                    .to_owned();
                if line.is_empty() {
                    "unknown".into()
                } else {
                    line
                }
            }
            Err(_) => "unavailable".into(),
        }
    }
    pub fn selected(&self, tool: Tool) -> Result<Option<String>> {
        let content = match fs::read_to_string(&self.config_path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        Ok(content
            .lines()
            .filter_map(|l| l.split_once('='))
            .find(|(key, _)| *key == tool.to_string())
            .map(|(_, v)| v.to_owned()))
    }
    pub fn select(&self, tool: Tool, version: &str) -> Result<()> {
        if version == "system" {
            if self.system(tool).is_empty() {
                return Err(format!("{tool} not found on PATH").into());
            }
        } else if let Some(name) = version.strip_prefix("local:") {
            validate_local_name(name)?;
            let path = self
                .local(tool)?
                .into_iter()
                .find(|(n, _)| n == name)
                .map(|(_, path)| path)
                .ok_or_else(|| format!("local {tool} {name} is not registered"))?;
            if !path.is_file() {
                return Err(format!("local {tool} {name} is missing: {}", path.display()).into());
            }
        } else {
            validate_version(version)?;
            if !self.executable(tool, version).is_file() {
                return Err(format!("{tool} {version} is not installed").into());
            }
        }
        let mut lines = match fs::read_to_string(&self.config_path) {
            Ok(s) => s
                .lines()
                .filter(|l| !l.starts_with(&format!("{tool}=")))
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => vec![],
            Err(e) => return Err(e.into()),
        };
        lines.push(format!("{tool}={version}"));
        fs::create_dir_all(self.config_path.parent().ok_or("invalid config path")?)?;
        fs::write(&self.config_path, format!("{}\n", lines.join("\n")))?;
        Ok(())
    }
    pub fn resolve(&self, tool: Tool) -> Result<PathBuf> {
        match self.selected(tool)?.as_deref() {
            Some("system") | None => self
                .system(tool)
                .into_iter()
                .next()
                .ok_or_else(|| format!("{tool} not found on PATH").into()),
            Some(v) if v.starts_with("local:") => {
                let name = &v[6..];
                validate_local_name(name)?;
                let path = self
                    .local(tool)?
                    .into_iter()
                    .find(|(n, _)| n == name)
                    .map(|(_, path)| path)
                    .ok_or_else(|| format!("local {tool} {name} is not registered"))?;
                if path.is_file() {
                    Ok(path)
                } else {
                    Err(format!(
                        "selected local {tool} {name} is missing: {}",
                        path.display()
                    )
                    .into())
                }
            }
            Some(v) => {
                validate_version(v)?;
                let path = self.executable(tool, v);
                if path.is_file() {
                    Ok(path)
                } else {
                    Err(format!("selected {tool} {v} is missing").into())
                }
            }
        }
    }
    pub fn uninstall(&self, tool: Tool, version: &str) -> Result<()> {
        validate_version(version)?;
        let tool_dir = self.data_dir.join(tool.to_string());
        let root = tool_dir.join(version);
        let metadata = fs::symlink_metadata(&root).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                format!("{tool} {version} is not installed")
            } else {
                format!("cannot inspect {}: {e}", root.display())
            }
        })?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("installation path is not a regular directory".into());
        }
        let data_dir = fs::canonicalize(&self.data_dir)?;
        let canonical_tool = fs::canonicalize(&tool_dir)?;
        let canonical_root = fs::canonicalize(&root)?;
        if canonical_tool.parent() != Some(data_dir.as_path())
            || canonical_tool.file_name() != Some(std::ffi::OsStr::new(&tool.to_string()))
            || canonical_root.parent() != Some(canonical_tool.as_path())
            || canonical_root.file_name() != Some(std::ffi::OsStr::new(version))
        {
            return Err("installation path resolves outside its managed directory".into());
        }

        // Read the configuration before removing files so a read error cannot leave
        // an otherwise avoidable stale selection behind.
        let config = match fs::read_to_string(&self.config_path) {
            Ok(content) => Some(content),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        let key = format!("{tool}=");
        let selected = config
            .as_deref()
            .and_then(|content| content.lines().find_map(|line| line.strip_prefix(&key)))
            == Some(version);
        fs::remove_dir_all(&root)?;
        if selected {
            let remaining = config
                .unwrap_or_default()
                .lines()
                .filter(|line| !line.starts_with(&key))
                .collect::<Vec<_>>()
                .join("\n");
            let updated = if remaining.is_empty() {
                String::new()
            } else {
                format!("{remaining}\n")
            };
            fs::write(&self.config_path, updated).map_err(|e| {
                format!("{tool} {version} was removed, but its selection could not be cleared: {e}")
            })?;
            println!(
                "removed {tool} {version}; selection cleared (PATH will be used if available)"
            );
        } else {
            println!("removed {tool} {version}");
        }
        Ok(())
    }
    pub fn install_latest(&self, tool: Tool) -> Result<()> {
        self.install_latest_cancellable(tool, &AtomicBool::new(false))
    }
    pub fn install_latest_cancellable(&self, tool: Tool, cancel: &AtomicBool) -> Result<()> {
        self.install_latest_with_progress(tool, cancel, &Default::default())
    }
    pub fn install_latest_with_progress(
        &self,
        tool: Tool,
        cancel: &AtomicBool,
        progress: &SharedInstallProgress,
    ) -> Result<()> {
        stage(progress, "正在查询最新版本…");
        eprintln!("Looking up latest {tool} release...");
        let url = match tool {
            Tool::OpenOcd => {
                "https://api.github.com/repos/xpack-dev-tools/openocd-xpack/releases?per_page=100"
            }
            Tool::PyOcd => "https://pypi.org/pypi/pyocd/json",
            Tool::ProbeRs => "https://crates.io/api/v1/crates/probe-rs-tools",
        };
        let mut command = Command::new("curl");
        command
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--proto",
                "=https",
                "--tlsv1.2",
                "--user-agent",
                "nsc/0.1",
                url,
            ])
            .stdout(std::process::Stdio::piped());
        // Drain the response concurrently so a large release list cannot fill the pipe.
        let output = run_capture(&mut command, cancel)?;
        let metadata: serde_json::Value = serde_json::from_slice(&output)?;
        let platform = if tool == Tool::OpenOcd {
            Some(openocd_platform()?)
        } else {
            None
        };
        let version = latest_version(tool, &metadata, platform)?;
        eprintln!("Latest {tool}: {version}");
        self.install_with_progress(tool, &version, cancel, progress)
    }
    pub fn install(&self, tool: Tool, version: &str) -> Result<()> {
        self.install_cancellable(tool, version, &AtomicBool::new(false))
    }
    pub fn install_cancellable(
        &self,
        tool: Tool,
        version: &str,
        cancel: &AtomicBool,
    ) -> Result<()> {
        self.install_with_progress(tool, version, cancel, &Default::default())
    }
    pub fn install_with_progress(
        &self,
        tool: Tool,
        version: &str,
        cancel: &AtomicBool,
        progress: &SharedInstallProgress,
    ) -> Result<()> {
        check_cancel(cancel)?;
        validate_version(version)?;
        let root = self.data_dir.join(tool.to_string()).join(version);
        if root.exists() {
            return Err(
                format!("installation directory already exists: {}", root.display()).into(),
            );
        }
        fs::create_dir_all(root.parent().ok_or("invalid data path")?)?;
        let result = (|| -> Result<()> {
            match tool {
                Tool::PyOcd => {
                    stage(progress, "正在创建 Python 环境…");
                    let python = if cfg!(windows) { "python" } else { "python3" };
                    run_cancellable(
                        Command::new(python).arg("-m").arg("venv").arg(&root),
                        cancel,
                    )?;
                    let interpreter = if cfg!(windows) {
                        root.join("Scripts/python.exe")
                    } else {
                        root.join("bin/python")
                    };
                    stage(progress, "正在准备下载 Python 依赖…");
                    let help = run_capture(
                        Command::new(&interpreter)
                            .args(["-m", "pip", "install", "--help"])
                            .stdout(std::process::Stdio::piped()),
                        cancel,
                    )?;
                    let raw = String::from_utf8_lossy(&help).contains("on, off, raw");
                    run_monitored(
                        Command::new(interpreter).args([
                            "-u",
                            "-m",
                            "pip",
                            "install",
                            "--progress-bar",
                            if raw { "raw" } else { "on" },
                            &format!("pyocd=={version}"),
                        ]),
                        cancel,
                        progress,
                        MonitorKind::Pip,
                    )?;
                }
                Tool::ProbeRs => {
                    stage(progress, "正在准备下载 Rust 依赖…");
                    run_monitored(
                        Command::new("cargo")
                            .env("CARGO_TERM_COLOR", "never")
                            .env("CARGO_TERM_PROGRESS_WHEN", "always")
                            // Cargo requires an explicit width when progress is forced.
                            .env("CARGO_TERM_PROGRESS_WIDTH", "80")
                            .args([
                                "install",
                                "probe-rs-tools",
                                "--locked",
                                "--version",
                                &format!("={version}"),
                                "--root",
                            ])
                            .arg(&root),
                        cancel,
                        progress,
                        MonitorKind::Cargo,
                    )?;
                }
                Tool::OpenOcd => {
                    let platform = openocd_platform()?;
                    fs::create_dir_all(&root)?;
                    let archive = root.join("download.archive");
                    let url = format!("https://github.com/xpack-dev-tools/openocd-xpack/releases/download/v{version}/xpack-openocd-{version}-{platform}");
                    stage(progress, "正在下载 OpenOCD…");
                    run_download(
                        Command::new("curl")
                            .args([
                                "--fail",
                                "--location",
                                "--proto",
                                "=https",
                                "--tlsv1.2",
                                "--output",
                            ])
                            .arg(&archive)
                            .arg(url),
                        cancel,
                        progress,
                    )?;
                    stage(progress, "下载完成，正在解压…");
                    run_cancellable(
                        Command::new("tar")
                            .arg("-xf")
                            .arg(&archive)
                            .arg("-C")
                            .arg(&root),
                        cancel,
                    )?;
                    fs::remove_file(archive)?;
                }
            }
            check_cancel(cancel)?;
            if !self.executable(tool, version).is_file() {
                return Err("installation did not produce the expected executable".into());
            }
            Ok(())
        })();
        if result.is_err() && root.exists() {
            let _ = fs::remove_dir_all(&root);
        }
        result?;
        println!("installed {tool} {version}; select it with: use {tool} {version}");
        Ok(())
    }
}
fn openocd_platform() -> Result<&'static str> {
    match (env::consts::OS, env::consts::ARCH) {
        ("windows", "x86_64") => Ok("win32-x64.zip"),
        ("linux", "x86_64") => Ok("linux-x64.tar.gz"),
        ("linux", "aarch64") => Ok("linux-arm64.tar.gz"),
        ("macos", "x86_64") => Ok("darwin-x64.tar.gz"),
        ("macos", "aarch64") => Ok("darwin-arm64.tar.gz"),
        _ => Err("no supported xPack OpenOCD archive for this platform".into()),
    }
}

fn latest_version(
    tool: Tool,
    metadata: &serde_json::Value,
    platform: Option<&str>,
) -> Result<String> {
    let version = match tool {
        Tool::PyOcd => metadata.pointer("/info/version").and_then(|v| v.as_str()),
        Tool::ProbeRs => metadata
            .pointer("/crate/max_stable_version")
            .and_then(|v| v.as_str()),
        Tool::OpenOcd => {
            let suffix = format!("-{}", platform.ok_or("missing platform")?);
            metadata.as_array().and_then(|releases| {
                releases
                    .iter()
                    .filter(|release| release["draft"] == false && release["prerelease"] == false)
                    .find_map(|release| {
                        let tag = release["tag_name"].as_str()?.strip_prefix('v')?;
                        let expected = format!("xpack-openocd-{tag}{suffix}");
                        release["assets"]
                            .as_array()?
                            .iter()
                            .any(|asset| asset["name"].as_str() == Some(expected.as_str()))
                            .then_some(tag)
                    })
            })
        }
    }
    .ok_or("no suitable stable release found in registry response")?;
    validate_version(version)?;
    Ok(version.to_owned())
}
fn validate_version(version: &str) -> Result<()> {
    if version.is_empty()
        || version.len() > 100
        || !version.as_bytes()[0].is_ascii_digit()
        || !version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'_')
    {
        return Err("invalid version; use a concrete version beginning with a digit".into());
    }
    Ok(())
}
fn validate_local_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 100
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    {
        return Err("invalid local name; use letters, digits, '.', '-' or '_'".into());
    }
    Ok(())
}
fn check_cancel(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        Err("安装已取消".into())
    } else {
        Ok(())
    }
}

fn spawn_install(command: &mut Command, cancel: &AtomicBool) -> Result<std::process::Child> {
    check_cancel(cancel)?;
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    Ok(command.spawn()?)
}

fn wait_install(child: &mut std::process::Child, cancel: &AtomicBool) -> Result<()> {
    loop {
        if cancel.load(Ordering::SeqCst) {
            #[cfg(unix)]
            {
                let _ = Command::new("kill")
                    .args(["-KILL", "--", &format!("-{}", child.id())])
                    .status();
            }
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &child.id().to_string()])
                    .status();
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err("安装已取消".into());
        }
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("安装命令失败：{status}").into())
            };
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run_cancellable(command: &mut Command, cancel: &AtomicBool) -> Result<()> {
    let mut child = spawn_install(command, cancel)?;
    wait_install(&mut child, cancel)
}

#[derive(Clone, Copy)]
enum MonitorKind {
    Pip,
    Cargo,
}

struct ProgressMonitor {
    kind: MonitorKind,
    started: Instant,
    first_bytes: u64,
    last_bytes: u64,
    last_total: u64,
    downloads: usize,
    building: bool,
    build_sample: Option<(Instant, u64)>,
}

impl ProgressMonitor {
    fn new(kind: MonitorKind) -> Self {
        Self {
            kind,
            started: Instant::now(),
            first_bytes: 0,
            last_bytes: 0,
            last_total: 0,
            downloads: 0,
            building: false,
            build_sample: None,
        }
    }
    fn update(&mut self, line: &str) -> Option<InstallProgress> {
        let line = line.trim();
        match self.kind {
            MonitorKind::Pip => {
                let fields: Vec<_> = line.split_whitespace().collect();
                if let ["Progress", current, "of", total] = fields.as_slice() {
                    let current: u64 = current.parse().ok()?;
                    let total: u64 = total.parse().ok()?;
                    if current < self.last_bytes || total != self.last_total {
                        self.started = Instant::now();
                        self.first_bytes = current;
                    }
                    self.last_bytes = current;
                    self.last_total = total;
                    let elapsed = self.started.elapsed().as_secs_f64();
                    let speed = if elapsed >= 0.2 && current > self.first_bytes {
                        Some(((current - self.first_bytes) as f64 / elapsed) as u64)
                    } else {
                        None
                    };
                    let remaining = speed.filter(|speed| *speed > 0).and_then(|speed| {
                        if total > 0 && current <= total {
                            Some((total - current) / speed)
                        } else {
                            None
                        }
                    });
                    return Some(InstallProgress {
                        stage: "正在下载 Python 依赖（当前文件）…".into(),
                        percent: if total > 0 {
                            Some((current as f64 / total as f64 * 100.0).min(100.0) as f32)
                        } else {
                            None
                        },
                        downloaded: Some(current),
                        total: if total > 0 { Some(total) } else { None },
                        bytes_per_second: speed,
                        remaining_seconds: remaining,
                    });
                }
                if line.starts_with("Downloading ") || line.starts_with("Using cached ") {
                    self.last_bytes = 0;
                    self.last_total = 0;
                    return Some(InstallProgress {
                        stage: "正在获取 Python 依赖（当前文件）…".into(),
                        ..Default::default()
                    });
                }
                if line.starts_with("Installing collected packages:")
                    || line.starts_with("Building wheels for collected packages:")
                {
                    return Some(InstallProgress {
                        stage: "正在构建或安装 Python 依赖…".into(),
                        ..Default::default()
                    });
                }
            }
            MonitorKind::Cargo => {
                if line.starts_with("Downloaded ") {
                    self.downloads += 1;
                    return Some(InstallProgress {
                        stage: format!("正在下载 Rust 依赖（已下载 {} 个包）…", self.downloads),
                        ..Default::default()
                    });
                }
                if line.starts_with("Downloading ") {
                    return Some(InstallProgress {
                        stage: format!("正在下载 Rust 依赖（已下载 {} 个包）…", self.downloads),
                        ..Default::default()
                    });
                }
                if let Some(rest) = line.strip_prefix("Building ") {
                    self.building = true;
                    if let Some((_, tail)) = rest.split_once(']') {
                        if let Some((done, total)) = tail
                            .split_whitespace()
                            .next()
                            .and_then(|part| part.split_once('/'))
                        {
                            if let (Ok(done), Ok(total)) = (
                                done.parse::<u64>(),
                                total.trim_end_matches(':').parse::<u64>(),
                            ) {
                                if total > 0 {
                                    let (start, initial) =
                                        *self.build_sample.get_or_insert((Instant::now(), done));
                                    let elapsed = start.elapsed().as_secs_f64();
                                    let completed = done.saturating_sub(initial);
                                    let remaining_seconds =
                                        if completed > 0 && elapsed >= 1.0 && done <= total {
                                            Some(
                                                ((total - done) as f64 * elapsed / completed as f64)
                                                    as u64,
                                            )
                                        } else {
                                            None
                                        };
                                    return Some(InstallProgress {
                                        stage: format!(
                                            "正在编译 Rust 依赖（{done}/{total} 个任务）…"
                                        ),
                                        percent: Some(
                                            (done as f64 / total as f64 * 100.0).min(100.0) as f32,
                                        ),
                                        remaining_seconds,
                                        ..Default::default()
                                    });
                                }
                            }
                        }
                    }
                }
                if line.starts_with("Compiling ") && !self.building {
                    self.building = true;
                    return Some(InstallProgress {
                        stage: "正在编译 Rust 依赖…".into(),
                        ..Default::default()
                    });
                }
            }
        }
        None
    }
}

fn run_monitored(
    command: &mut Command,
    cancel: &AtomicBool,
    progress: &SharedInstallProgress,
    kind: MonitorKind,
) -> Result<()> {
    use std::io::{Read, Write};
    match kind {
        MonitorKind::Pip => {
            command.stdout(std::process::Stdio::piped());
        }
        MonitorKind::Cargo => {
            command.stderr(std::process::Stdio::piped());
        }
    }
    let mut child = spawn_install(command, cancel)?;
    let mut stream: Box<dyn Read + Send> = match kind {
        MonitorKind::Pip => Box::new(child.stdout.take().ok_or("无法读取 pip 输出")?),
        MonitorKind::Cargo => Box::new(child.stderr.take().ok_or("无法读取 Cargo 输出")?),
    };
    let shared = progress.clone();
    let reader = std::thread::spawn(move || {
        let mut buffer = [0; 4096];
        let mut line = Vec::new();
        let mut monitor = ProgressMonitor::new(kind);
        loop {
            let count = match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            match kind {
                MonitorKind::Pip => {
                    let _ = std::io::stdout().write_all(&buffer[..count]);
                }
                MonitorKind::Cargo => {
                    let _ = std::io::stderr().write_all(&buffer[..count]);
                }
            }
            for byte in &buffer[..count] {
                if *byte == b'\r' || *byte == b'\n' {
                    let text = String::from_utf8_lossy(&line);
                    // Cargo prefixes terminal updates with an ANSI erase-line sequence.
                    let text = text
                        .trim_start()
                        .trim_start_matches("\x1b[K")
                        .trim_start_matches("\x1b[2K");
                    if let Some(value) = monitor.update(text) {
                        if let Ok(mut current) = shared.lock() {
                            *current = value;
                        }
                    }
                    line.clear();
                } else if line.len() < 8192 {
                    line.push(*byte);
                }
            }
        }
    });
    let result = wait_install(&mut child, cancel);
    let _ = reader.join();
    result
}

fn parse_size(value: &str) -> Option<u64> {
    let (number, multiplier) = match value.chars().last()? {
        'k' | 'K' => (&value[..value.len() - 1], 1024_f64),
        'M' => (&value[..value.len() - 1], 1024_f64.powi(2)),
        'G' => (&value[..value.len() - 1], 1024_f64.powi(3)),
        'T' => (&value[..value.len() - 1], 1024_f64.powi(4)),
        _ => (value, 1.0),
    };
    let number = number.parse::<f64>().ok()?;
    if !number.is_finite() || number < 0.0 {
        return None;
    }
    Some((number * multiplier) as u64)
}

fn parse_remaining(value: &str) -> Option<u64> {
    let parts = value
        .split(':')
        .map(str::parse::<u64>)
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    match parts.as_slice() {
        [h, m, s] if *m < 60 && *s < 60 => Some(h * 3600 + m * 60 + s),
        _ => None,
    }
}

fn parse_curl_progress(line: &str) -> Option<InstallProgress> {
    let fields: Vec<_> = line.split_whitespace().collect();
    if fields.len() != 12 {
        return None;
    }
    let percent = fields[2].parse::<f32>().ok()?;
    let total = parse_size(fields[1])?;
    let downloaded = parse_size(fields[3])?;
    Some(InstallProgress {
        stage: "正在下载 OpenOCD…".into(),
        percent: if total > 0 {
            Some(percent.clamp(0.0, 100.0))
        } else {
            None
        },
        downloaded: Some(downloaded),
        total: if total > 0 { Some(total) } else { None },
        bytes_per_second: parse_size(fields[11]),
        remaining_seconds: parse_remaining(fields[10]),
    })
}

fn run_download(
    command: &mut Command,
    cancel: &AtomicBool,
    progress: &SharedInstallProgress,
) -> Result<()> {
    use std::io::{Read, Write};
    command.stderr(std::process::Stdio::piped());
    let mut child = spawn_install(command, cancel)?;
    let mut stderr = child.stderr.take().ok_or("无法读取下载进度")?;
    let shared = progress.clone();
    let reader = std::thread::spawn(move || {
        let mut buffer = [0; 4096];
        let mut line = Vec::new();
        loop {
            let count = match stderr.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let _ = std::io::stderr().write_all(&buffer[..count]);
            for byte in &buffer[..count] {
                if *byte == b'\r' || *byte == b'\n' {
                    if let Some(value) = parse_curl_progress(&String::from_utf8_lossy(&line)) {
                        if let Ok(mut current) = shared.lock() {
                            *current = value;
                        }
                    }
                    line.clear();
                } else if line.len() < 8192 {
                    line.push(*byte);
                }
            }
        }
    });
    let result = wait_install(&mut child, cancel);
    let _ = reader.join();
    result
}

fn run_capture(command: &mut Command, cancel: &AtomicBool) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut child = spawn_install(command, cancel)?;
    let mut stdout = child.stdout.take().ok_or("无法读取安装命令输出")?;
    let reader = std::thread::spawn(move || {
        let mut data = Vec::new();
        stdout.read_to_end(&mut data).map(|_| data)
    });
    let result = wait_install(&mut child, cancel);
    let output = reader.join().map_err(|_| "无法读取安装命令输出")??;
    result?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_reports_each_download_and_resets_measurements() {
        let mut monitor = ProgressMonitor::new(MonitorKind::Pip);
        monitor.update("  Downloading example.whl").unwrap();
        let first = monitor.update("Progress 100 of 1000").unwrap();
        assert_eq!(first.percent, Some(10.0));
        monitor.started = Instant::now() - Duration::from_secs(2);
        let next = monitor.update("Progress 500 of 1000").unwrap();
        assert!((198..=200).contains(&next.bytes_per_second.unwrap()));
        assert_eq!(next.remaining_seconds, Some(2));
        monitor.update("  Downloading another.whl").unwrap();
        let next_file = monitor.update("Progress 50 of 500").unwrap();
        assert_eq!(next_file.percent, Some(10.0));
        assert_eq!(next_file.bytes_per_second, None);
        let installing = monitor
            .update("Installing collected packages: pyocd")
            .unwrap();
        assert_eq!(installing.percent, None);
    }

    #[test]
    fn cargo_reports_download_count_and_build_progress() {
        let mut monitor = ProgressMonitor::new(MonitorKind::Cargo);
        assert!(monitor
            .update("  Downloaded foo v1.0.0")
            .unwrap()
            .stage
            .contains("1 个包"));
        assert!(monitor
            .update("  Downloaded bar v2.0.0")
            .unwrap()
            .stage
            .contains("2 个包"));
        let build = monitor
            .update("    Building [====>    ] 12/48: foo")
            .unwrap();
        assert_eq!(build.percent, Some(25.0));
        assert!(build.stage.contains("12/48"));
        assert!(monitor.update("   Compiling foo v1.0.0").is_none());
        assert!(monitor.update("network warning").is_none());
    }

    #[test]
    fn curl_progress_reports_download_and_unknown_totals() {
        let progress = parse_curl_progress(
            " 25  100M   25 25.0M    0     0  2500k      0  0:00:40  0:00:10  0:00:30 2600k",
        )
        .unwrap();
        assert_eq!(progress.percent, Some(25.0));
        assert_eq!(progress.downloaded, Some(25 * 1024 * 1024));
        assert_eq!(progress.total, Some(100 * 1024 * 1024));
        assert_eq!(progress.bytes_per_second, Some(2600 * 1024));
        assert_eq!(progress.remaining_seconds, Some(30));
        let unknown = parse_curl_progress(
            "  0     0    0  123k    0     0  1200k      0 --:--:--  0:00:01 --:--:-- 1200k",
        )
        .unwrap();
        assert_eq!(unknown.percent, None);
        assert_eq!(unknown.remaining_seconds, None);
        assert!(parse_curl_progress("curl: network error").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_stops_install_process_group() {
        use std::sync::Arc;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let worker = std::thread::spawn(move || {
            run_cancellable(
                Command::new("sh").args(["-c", "sleep 30 & wait"]),
                &worker_cancel,
            )
            .map_err(|e| e.to_string())
        });
        std::thread::sleep(Duration::from_millis(200));
        cancel.store(true, Ordering::SeqCst);
        assert!(worker.join().unwrap().unwrap_err().contains("安装已取消"));
    }

    #[test]
    fn cancelled_install_does_not_create_directory() {
        let root = std::env::temp_dir().join(format!("nsc-cancel-test-{}", std::process::id()));
        let manager = Manager {
            config_path: root.join("config"),
            data_dir: root.join("data"),
        };
        let cancel = AtomicBool::new(true);
        assert!(manager
            .install_cancellable(Tool::PyOcd, "1.0", &cancel)
            .is_err());
        assert!(!root.exists());
    }

    #[test]
    fn latest_versions_use_stable_registry_metadata() {
        let py = serde_json::json!({"info": {"version": "0.40.0"}});
        let probe = serde_json::json!({"crate": {"max_stable_version": "0.29.0"}});
        assert_eq!(latest_version(Tool::PyOcd, &py, None).unwrap(), "0.40.0");
        assert_eq!(
            latest_version(Tool::ProbeRs, &probe, None).unwrap(),
            "0.29.0"
        );
        let releases = serde_json::json!([
            {"tag_name":"v0.12.0-8", "draft":false, "prerelease":true, "assets":[{"name":"xpack-openocd-0.12.0-8-win32-x64.zip"}]},
            {"tag_name":"v0.12.0-7.1", "draft":false, "prerelease":false, "assets":[]},
            {"tag_name":"v0.12.0-7", "draft":false, "prerelease":false, "assets":[{"name":"xpack-openocd-0.12.0-7-win32-x64.zip"}]}
        ]);
        assert_eq!(
            latest_version(Tool::OpenOcd, &releases, Some("win32-x64.zip")).unwrap(),
            "0.12.0-7"
        );
        assert!(latest_version(Tool::OpenOcd, &releases, Some("linux-x64.tar.gz")).is_err());
    }
    #[test]
    fn versions_are_safe_path_components() {
        for v in ["", "../x", "1/../../x", "1\\x", "latest", "1=2"] {
            assert!(validate_version(v).is_err(), "{v}");
        }
        for v in ["0.12.0-7", "0.31.0", "1.0rc1"] {
            assert!(validate_version(v).is_ok());
        }
    }
    #[test]
    fn tool_alias() {
        assert_eq!("probers".parse::<Tool>().unwrap(), Tool::ProbeRs);
    }
    #[test]
    fn local_registration_selection_and_removal() {
        let root = std::env::temp_dir().join(format!("nya-local-test-{}", std::process::id()));
        let manager = Manager {
            config_path: root.join("config/config"),
            data_dir: root.join("data"),
        };
        let dir = root.join("my debugger");
        let executable = dir.join("bin").join(if cfg!(windows) {
            "openocd.exe"
        } else {
            "openocd"
        });
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "test").unwrap();
        assert!(manager.add_local(Tool::PyOcd, "other", &dir).is_err());
        assert!(manager.add_local(Tool::OpenOcd, "bad:name", &dir).is_err());
        manager.add_local(Tool::OpenOcd, "custom", &dir).unwrap();
        assert!(manager.add_local(Tool::OpenOcd, "custom", &dir).is_err());
        manager.select(Tool::OpenOcd, "local:custom").unwrap();
        assert_eq!(
            manager.resolve(Tool::OpenOcd).unwrap(),
            fs::canonicalize(&dir)
                .unwrap()
                .join("bin")
                .join(executable.file_name().unwrap())
        );
        fs::remove_file(&executable).unwrap();
        assert!(manager.resolve(Tool::OpenOcd).is_err());
        fs::write(&executable, "test").unwrap();
        manager.remove_local(Tool::OpenOcd, "custom").unwrap();
        assert!(executable.exists());
        assert_eq!(manager.selected(Tool::OpenOcd).unwrap(), None);
        assert!(manager.local(Tool::OpenOcd).unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn selection_persists_and_resolves() {
        let root = std::env::temp_dir().join(format!("nya-manager-test-{}", std::process::id()));
        let manager = Manager {
            config_path: root.join("config/config"),
            data_dir: root.join("data"),
        };
        let executable = manager.executable(Tool::OpenOcd, "0.12.0-7");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "test").unwrap();
        manager.select(Tool::OpenOcd, "0.12.0-7").unwrap();
        assert_eq!(
            manager.selected(Tool::OpenOcd).unwrap().as_deref(),
            Some("0.12.0-7")
        );
        assert_eq!(manager.resolve(Tool::OpenOcd).unwrap(), executable);
        assert_eq!(manager.installed(Tool::OpenOcd).unwrap().len(), 1);
        assert!(manager.select(Tool::PyOcd, "0.0.0").is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn uninstall_removes_only_requested_version_and_clears_its_selection() {
        let root = std::env::temp_dir().join(format!("nya-uninstall-test-{}", std::process::id()));
        let manager = Manager {
            config_path: root.join("config/config"),
            data_dir: root.join("data"),
        };
        for (tool, version) in [
            (Tool::OpenOcd, "1.0"),
            (Tool::OpenOcd, "2.0"),
            (Tool::PyOcd, "1.0"),
        ] {
            let executable = manager.executable(tool, version);
            fs::create_dir_all(executable.parent().unwrap()).unwrap();
            fs::write(executable, "test").unwrap();
        }
        manager.select(Tool::OpenOcd, "1.0").unwrap();
        manager.select(Tool::PyOcd, "1.0").unwrap();
        manager.uninstall(Tool::OpenOcd, "2.0").unwrap();
        assert_eq!(
            manager.selected(Tool::OpenOcd).unwrap().as_deref(),
            Some("1.0")
        );
        manager.uninstall(Tool::OpenOcd, "1.0").unwrap();
        assert_eq!(manager.selected(Tool::OpenOcd).unwrap(), None);
        assert_eq!(
            manager.selected(Tool::PyOcd).unwrap().as_deref(),
            Some("1.0")
        );
        assert!(manager.executable(Tool::PyOcd, "1.0").exists());
        assert!(manager.uninstall(Tool::OpenOcd, "1.0").is_err());
        assert!(manager.uninstall(Tool::OpenOcd, "../config").is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
