//! Absolute user directories shared by CortexKit tools.
//!
//! Unix (including macOS) uses the XDG layout, not `~/Library`: absolute,
//! non-empty XDG values win, then the user's home directory supplies the default.
//! Windows also honors XDG overrides, but otherwise uses roaming `APPDATA` for
//! config and local `LOCALAPPDATA` for data, state, and cache. Large databases
//! should not roam with a domain profile.
//!
//! # Existing Windows installations
//!
//! [`cortexkit_data_dir`] preserves an existing `APPDATA/cortexkit` directory
//! when the local `cortexkit` directory does not exist. An XDG data override
//! disables this legacy lookup. No files are moved, merged, or created. The run
//! directory follows the selected data directory. Without a legacy installation,
//! Windows uses local data, unlike older daemons that always used roaming data.
//!
//! # Joining paths
//!
//! Paths are assembled with [`Path::join`] or [`PathBuf::push`], never by adding
//! slash-delimited strings. Hard-coded `/` produces mixed separators on Windows.
//! A path below a regular file fails with "not a directory" on Unix but can fail
//! with "not found" on Windows; treating `NotFound` as "absent" can therefore
//! silently skip that path. Resolution does not assert that a directory exists
//! or is writable; callers must still handle filesystem errors when opening it.

use std::{
    env,
    error::Error,
    ffi::OsString,
    fmt, io,
    path::{Path, PathBuf},
};

/// Environment and filesystem inputs used by a directory resolver.
///
/// Override the filesystem methods as well as `var_os` for fully isolated tests.
/// No resolver mutates the process environment.
pub trait EnvSource {
    /// Read a variable without losing non-Unicode path characters.
    fn var_os(&self, key: &str) -> Option<OsString>;

    /// Whether a legacy or local data directory already exists.
    fn path_exists(&self, path: &Path) -> bool {
        path.exists()
    }

    /// The temporary directory used by connection-file discovery.
    fn temp_dir(&self) -> PathBuf {
        env::temp_dir()
    }

    /// Base for an explicit relative connection-file argument.
    fn current_dir(&self) -> io::Result<PathBuf> {
        env::current_dir()
    }
}

/// Inputs from the current process and filesystem.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemEnv;

impl EnvSource for SystemEnv {
    fn var_os(&self, key: &str) -> Option<OsString> {
        env::var_os(key)
    }
}

/// A path could not be resolved without an unsafe relative fallback.
#[derive(Debug)]
pub enum PathsError {
    /// None of the listed variables supplied a usable absolute directory.
    Unresolved {
        /// The directory being resolved.
        directory: &'static str,
        /// Variables considered, in priority order.
        variables: &'static [&'static str],
    },
    /// An exclusive connection-file override or base directory was relative.
    RelativePath {
        /// The input that supplied the path.
        source: String,
        /// The rejected path.
        path: PathBuf,
    },
    /// The current directory could not be read for an explicit relative path.
    CurrentDir(io::Error),
    /// A per-user token was empty or contained a path separator.
    InvalidUserToken,
}

impl fmt::Display for PathsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unresolved {
                directory,
                variables,
            } => write!(
                f,
                "could not resolve an absolute {directory}; tried {}",
                variables.join(", ")
            ),
            Self::RelativePath { source, path } => {
                write!(f, "{source} must be absolute, got {}", path.display())
            }
            Self::CurrentDir(error) => write!(f, "could not resolve current directory: {error}"),
            Self::InvalidUserToken => f.write_str(
                "connection-file user token must be non-empty and contain no path separators",
            ),
        }
    }
}

impl Error for PathsError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CurrentDir(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy)]
enum Platform {
    Unix,
    Windows,
}

/// A resolver borrowing injectable inputs. Construct it with [`with_env`].
pub struct Dirs<'a, E: EnvSource + ?Sized> {
    env: &'a E,
    platform: Platform,
}

/// Resolve directories using the supplied inputs on the current platform.
pub fn with_env<E: EnvSource + ?Sized>(env: &E) -> Dirs<'_, E> {
    Dirs {
        env,
        platform: if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Unix
        },
    }
}

impl<E: EnvSource + ?Sized> Dirs<'_, E> {
    fn non_empty(&self, key: &str) -> Option<OsString> {
        self.env.var_os(key).filter(|value| !value.is_empty())
    }

    fn absolute_var(&self, key: &str) -> Option<PathBuf> {
        self.non_empty(key)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    }

    /// Unix `HOME`, or Windows `USERPROFILE`, then `HOMEDRIVE` + `HOMEPATH`,
    /// then `HOME`. `USERPROFILE` wins over Git Bash's `HOME` when both are set.
    /// Empty and relative values cannot supply a home directory.
    pub fn home(&self) -> Result<PathBuf, PathsError> {
        match self.platform {
            Platform::Unix => self.absolute_var("HOME").ok_or(PathsError::Unresolved {
                directory: "home directory",
                variables: &["HOME"],
            }),
            Platform::Windows => {
                if let Some(path) = self.absolute_var("USERPROFILE") {
                    return Ok(path);
                }
                if let (Some(drive), Some(path)) =
                    (self.non_empty("HOMEDRIVE"), self.non_empty("HOMEPATH"))
                {
                    let mut home = PathBuf::from(drive);
                    home.push(path);
                    if home.is_absolute() {
                        return Ok(home);
                    }
                }
                self.absolute_var("HOME").ok_or(PathsError::Unresolved {
                    directory: "home directory",
                    variables: &["USERPROFILE", "HOMEDRIVE", "HOMEPATH", "HOME"],
                })
            }
        }
    }

    fn base(
        &self,
        xdg: &'static str,
        windows: &'static str,
        unix_tail: &[&str],
        windows_tail: &str,
    ) -> Result<PathBuf, PathsError> {
        if let Some(path) = self.absolute_var(xdg) {
            return Ok(path);
        }
        match self.platform {
            Platform::Unix => {
                let mut path = self.home().map_err(|_| PathsError::Unresolved {
                    directory: xdg,
                    variables: match xdg {
                        "XDG_DATA_HOME" => &["XDG_DATA_HOME", "HOME"],
                        "XDG_CONFIG_HOME" => &["XDG_CONFIG_HOME", "HOME"],
                        "XDG_STATE_HOME" => &["XDG_STATE_HOME", "HOME"],
                        _ => &["XDG_CACHE_HOME", "HOME"],
                    },
                })?;
                for component in unix_tail {
                    path.push(component);
                }
                Ok(path)
            }
            Platform::Windows => {
                self.windows_base(windows, windows_tail)
                    .ok_or(PathsError::Unresolved {
                        directory: xdg,
                        variables: match xdg {
                            "XDG_DATA_HOME" => &["XDG_DATA_HOME", "LOCALAPPDATA", "USERPROFILE"],
                            "XDG_CONFIG_HOME" => &["XDG_CONFIG_HOME", "APPDATA", "USERPROFILE"],
                            "XDG_STATE_HOME" => &["XDG_STATE_HOME", "LOCALAPPDATA", "USERPROFILE"],
                            _ => &["XDG_CACHE_HOME", "LOCALAPPDATA", "USERPROFILE"],
                        },
                    })
            }
        }
    }

    fn windows_base(&self, variable: &str, tail: &str) -> Option<PathBuf> {
        self.absolute_var(variable).or_else(|| {
            self.absolute_var("USERPROFILE")
                .map(|path| path.join("AppData").join(tail))
        })
    }

    /// XDG data home, Unix `HOME/.local/share`, or Windows local application data.
    /// Unlike [`Self::xdg_data_home`], this uses `LOCALAPPDATA` on Windows when
    /// there is no absolute XDG override.
    pub fn data_home(&self) -> Result<PathBuf, PathsError> {
        self.base(
            "XDG_DATA_HOME",
            "LOCALAPPDATA",
            &[".local", "share"],
            "Local",
        )
    }

    /// XDG data layout on every OS: an absolute `XDG_DATA_HOME` override, then
    /// [`Self::home`] plus `.local/share`. Empty or relative overrides are ignored.
    /// Unlike [`Self::data_home`], this never consults `APPDATA` or `LOCALAPPDATA`;
    /// use it for applications that follow XDG layout even on Windows.
    pub fn xdg_data_home(&self) -> Result<PathBuf, PathsError> {
        if let Some(path) = self.absolute_var("XDG_DATA_HOME") {
            return Ok(path);
        }
        let home = self.home().map_err(|_| PathsError::Unresolved {
            directory: "XDG data home",
            variables: match self.platform {
                Platform::Unix => &["XDG_DATA_HOME", "HOME"],
                Platform::Windows => &[
                    "XDG_DATA_HOME",
                    "USERPROFILE",
                    "HOMEDRIVE",
                    "HOMEPATH",
                    "HOME",
                ],
            },
        })?;
        Ok(home.join(".local").join("share"))
    }

    /// XDG config home, Unix `HOME/.config`, or Windows roaming application data.
    pub fn config_home(&self) -> Result<PathBuf, PathsError> {
        self.base("XDG_CONFIG_HOME", "APPDATA", &[".config"], "Roaming")
    }

    /// XDG state home, Unix `HOME/.local/state`, or Windows local application data.
    pub fn state_home(&self) -> Result<PathBuf, PathsError> {
        self.base(
            "XDG_STATE_HOME",
            "LOCALAPPDATA",
            &[".local", "state"],
            "Local",
        )
    }

    /// XDG cache home, Unix `HOME/.cache`, or Windows local application data.
    pub fn cache_home(&self) -> Result<PathBuf, PathsError> {
        self.base("XDG_CACHE_HOME", "LOCALAPPDATA", &[".cache"], "Local")
    }

    /// Data home plus `cortexkit`, preserving an existing roaming Windows install
    /// only when no local install exists and no absolute XDG override is set.
    pub fn cortexkit_data_dir(&self) -> Result<PathBuf, PathsError> {
        let local = self.data_home().map(|home| home.join("cortexkit"));
        if matches!(self.platform, Platform::Windows)
            && self.absolute_var("XDG_DATA_HOME").is_none()
        {
            if let Some(roaming) = self.windows_base("APPDATA", "Roaming") {
                let legacy = roaming.join("cortexkit");
                if self.env.path_exists(&legacy)
                    && !local.as_ref().is_ok_and(|path| self.env.path_exists(path))
                {
                    return Ok(legacy);
                }
            }
        }
        local
    }

    /// Config home plus `cortexkit`.
    pub fn cortexkit_config_dir(&self) -> Result<PathBuf, PathsError> {
        Ok(self.config_home()?.join("cortexkit"))
    }

    /// Selected data directory plus `run`. On Windows this follows the existing
    /// roaming installation when no local installation exists; see
    /// [`Self::cortexkit_data_dir`].
    pub fn cortexkit_run_dir(&self) -> Result<PathBuf, PathsError> {
        Ok(self.cortexkit_data_dir()?.join("run"))
    }

    /// State home plus `cortexkit`.
    pub fn cortexkit_state_dir(&self) -> Result<PathBuf, PathsError> {
        Ok(self.state_home()?.join("cortexkit"))
    }

    /// Cache home plus `cortexkit`.
    pub fn cortexkit_cache_dir(&self) -> Result<PathBuf, PathsError> {
        Ok(self.cache_home()?.join("cortexkit"))
    }

    /// Ordered transport discovery candidates, without creating or reading files.
    ///
    /// An explicit path is exclusive. A relative explicit path (such as a user's
    /// `--subc ./conn.json`) is joined to the current directory, without requiring
    /// the file to exist. Otherwise a non-empty `SUBC_CONNECTION_FILE` is exclusive
    /// and must be absolute: a relative environment override is an error, not a
    /// reason to silently try another daemon.
    ///
    /// With no override, candidates are `XDG_RUNTIME_DIR/subc-connection.json`,
    /// `HOME/.local/share/cortexkit/run/subc-connection.json`, then the temporary
    /// directory's `subc-{per_user_token}.connection.json`. Empty or relative
    /// runtime and home values are skipped. The temporary directory must be
    /// absolute. The caller supplies the transport's per-user token, which must
    /// be non-empty and contain neither `/` nor `\`.
    ///
    /// This deliberately retains the transport's HOME-based discovery order even
    /// on Windows; it does not substitute the new local data directory.
    pub fn connection_file_candidates(
        &self,
        explicit: Option<&Path>,
        per_user_token: &str,
    ) -> Result<Vec<PathBuf>, PathsError> {
        if let Some(path) = explicit {
            if path.is_absolute() {
                return Ok(vec![path.to_path_buf()]);
            }
            let cwd = self.env.current_dir().map_err(PathsError::CurrentDir)?;
            return Ok(vec![require_absolute(
                require_absolute(cwd, "current directory")?.join(path),
                "explicit connection file",
            )?]);
        }
        if let Some(path) = self.non_empty("SUBC_CONNECTION_FILE") {
            return Ok(vec![require_absolute(
                PathBuf::from(path),
                "SUBC_CONNECTION_FILE",
            )?]);
        }
        if per_user_token.is_empty() || per_user_token.contains(['/', '\\']) {
            return Err(PathsError::InvalidUserToken);
        }
        let mut candidates = Vec::new();
        if let Some(path) = self.absolute_var("XDG_RUNTIME_DIR") {
            candidates.push(path.join("subc-connection.json"));
        }
        if let Some(path) = self.absolute_var("HOME") {
            candidates.push(
                path.join(".local")
                    .join("share")
                    .join("cortexkit")
                    .join("run")
                    .join("subc-connection.json"),
            );
        }
        candidates.push(
            require_absolute(self.env.temp_dir(), "temporary directory")?
                .join(format!("subc-{per_user_token}.connection.json")),
        );
        Ok(candidates)
    }
}

fn require_absolute(path: PathBuf, source: &str) -> Result<PathBuf, PathsError> {
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(PathsError::RelativePath {
            source: source.to_owned(),
            path,
        })
    }
}

/// Read a consumer's own path variable without consulting the process environment.
/// Unset or empty values return `None`; absolute values are preserved verbatim.
/// A relative value is an error naming `variable`, unlike XDG directory resolution,
/// which ignores relative overrides and tries the home-directory fallback.
pub fn env_absolute_path<E: EnvSource + ?Sized>(
    env: &E,
    variable: &str,
) -> Result<Option<PathBuf>, PathsError> {
    env.var_os(variable)
        .filter(|value| !value.is_empty())
        .map(|value| require_absolute(PathBuf::from(value), variable))
        .transpose()
}

macro_rules! process_dirs {
    ($($name:ident),+ $(,)?) => {$(
        #[doc = concat!("Resolve using the process environment; see [`Dirs::", stringify!($name), "`].")]
        pub fn $name() -> Result<PathBuf, PathsError> { with_env(&SystemEnv).$name() }
    )+};
}

process_dirs!(
    home,
    data_home,
    xdg_data_home,
    config_home,
    state_home,
    cache_home,
    cortexkit_data_dir,
    cortexkit_config_dir,
    cortexkit_run_dir,
    cortexkit_state_dir,
    cortexkit_cache_dir
);

/// Discover connection files using process inputs; see [`Dirs::connection_file_candidates`].
pub fn connection_file_candidates(
    explicit: Option<&Path>,
    per_user_token: &str,
) -> Result<Vec<PathBuf>, PathsError> {
    with_env(&SystemEnv).connection_file_candidates(explicit, per_user_token)
}

#[cfg(test)]
mod tests;
