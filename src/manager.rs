//! Installation and selection of per-user debugger versions.
use std::{
    env,
    error::Error,
    fmt, fs,
    path::{Path, PathBuf},
    process::Command,
    str::FromStr,
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

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
        eprintln!("Looking up latest {tool} release...");
        let url = match tool {
            Tool::OpenOcd => {
                "https://api.github.com/repos/xpack-dev-tools/openocd-xpack/releases?per_page=100"
            }
            Tool::PyOcd => "https://pypi.org/pypi/pyocd/json",
            Tool::ProbeRs => "https://crates.io/api/v1/crates/probe-rs-tools",
        };
        let output = Command::new("curl")
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
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "failed to query latest {tool} release: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let platform = if tool == Tool::OpenOcd {
            Some(openocd_platform()?)
        } else {
            None
        };
        let version = latest_version(tool, &metadata, platform)?;
        eprintln!("Latest {tool}: {version}");
        self.install(tool, &version)
    }
    pub fn install(&self, tool: Tool, version: &str) -> Result<()> {
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
                    let python = if cfg!(windows) { "python" } else { "python3" };
                    run(Command::new(python).arg("-m").arg("venv").arg(&root))?;
                    let interpreter = if cfg!(windows) {
                        root.join("Scripts/python.exe")
                    } else {
                        root.join("bin/python")
                    };
                    run(Command::new(interpreter).args([
                        "-m",
                        "pip",
                        "install",
                        "--progress-bar",
                        "on",
                        &format!("pyocd=={version}"),
                    ]))?;
                }
                Tool::ProbeRs => {
                    run(Command::new("cargo")
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
                        .arg(&root))?;
                }
                Tool::OpenOcd => {
                    let platform = openocd_platform()?;
                    fs::create_dir_all(&root)?;
                    let archive = root.join("download.archive");
                    let url = format!("https://github.com/xpack-dev-tools/openocd-xpack/releases/download/v{version}/xpack-openocd-{version}-{platform}");
                    run(Command::new("curl")
                        .args([
                            "--fail",
                            "--progress-bar",
                            "--location",
                            "--proto",
                            "=https",
                            "--tlsv1.2",
                            "--output",
                        ])
                        .arg(&archive)
                        .arg(url))?;
                    run(Command::new("tar")
                        .arg("-xf")
                        .arg(&archive)
                        .arg("-C")
                        .arg(&root))?;
                    fs::remove_file(archive)?;
                }
            }
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
fn run(command: &mut Command) -> Result<()> {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("{command:?} failed: {status}").into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
