use super::*;
use std::collections::{HashMap, HashSet};

struct TestEnv {
    vars: HashMap<String, OsString>,
    existing: HashSet<PathBuf>,
    temp: PathBuf,
    cwd: PathBuf,
    cwd_error: bool,
}

impl Default for TestEnv {
    fn default() -> Self {
        Self {
            vars: HashMap::new(),
            existing: HashSet::new(),
            temp: absolute("temp"),
            cwd: absolute("cwd"),
            cwd_error: false,
        }
    }
}

impl TestEnv {
    fn set(&mut self, key: &str, value: impl Into<OsString>) {
        self.vars.insert(key.to_owned(), value.into());
    }

    fn resolver(&self, platform: Platform) -> Dirs<'_, Self> {
        Dirs {
            env: self,
            platform,
        }
    }
}

impl EnvSource for TestEnv {
    fn var_os(&self, key: &str) -> Option<OsString> {
        self.vars.get(key).cloned()
    }

    fn path_exists(&self, path: &Path) -> bool {
        self.existing.contains(path)
    }

    fn temp_dir(&self) -> PathBuf {
        self.temp.clone()
    }

    fn current_dir(&self) -> io::Result<PathBuf> {
        if self.cwd_error {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "test current directory",
            ))
        } else {
            Ok(self.cwd.clone())
        }
    }
}

fn absolute(name: &str) -> PathBuf {
    let root = if cfg!(windows) {
        Path::new(r"C:\")
    } else {
        Path::new("/")
    };
    root.join(name)
}

#[derive(Clone, Copy, Debug)]
enum Value {
    Set,
    Empty,
    Relative,
    Unset,
}

const VALUES: [Value; 4] = [Value::Set, Value::Empty, Value::Relative, Value::Unset];
const XDG: [&str; 4] = [
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
];

fn xdg_inputs() -> impl Iterator<Item = [Value; 4]> {
    (0..256).map(|combination| std::array::from_fn(|i| VALUES[(combination >> (2 * i)) & 3]))
}

fn input(env: &mut TestEnv, key: &str, value: Value, path: &Path) {
    match value {
        Value::Set => env.set(key, path),
        Value::Empty => env.set(key, ""),
        Value::Relative => env.set(key, "relative"),
        Value::Unset => {}
    }
}

fn expected_xdg(value: Value, path: PathBuf, fallback: Option<PathBuf>) -> Option<PathBuf> {
    if matches!(value, Value::Set) {
        Some(path)
    } else {
        fallback
    }
}

fn check_directories(
    env: &TestEnv,
    platform: Platform,
    expected_home: Option<PathBuf>,
    bases: [Option<PathBuf>; 4],
) {
    let resolver = env.resolver(platform);
    let [data, config, state, cache] = bases;
    let xdg_expected = expected_xdg(
        if env
            .vars
            .get("XDG_DATA_HOME")
            .is_some_and(|v| Path::new(v).is_absolute())
        {
            Value::Set
        } else {
            Value::Unset
        },
        env.vars
            .get("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_default(),
        expected_home
            .as_ref()
            .map(|p| p.join(".local").join("share")),
    );
    let results = [
        ("xdg_data_home", resolver.xdg_data_home(), xdg_expected),
        ("home", resolver.home(), expected_home),
        ("data_home", resolver.data_home(), data.clone()),
        ("config_home", resolver.config_home(), config.clone()),
        ("state_home", resolver.state_home(), state.clone()),
        ("cache_home", resolver.cache_home(), cache.clone()),
        (
            "cortexkit_data_dir",
            resolver.cortexkit_data_dir(),
            data.clone().map(|p| p.join("cortexkit")),
        ),
        (
            "cortexkit_config_dir",
            resolver.cortexkit_config_dir(),
            config.map(|p| p.join("cortexkit")),
        ),
        (
            "cortexkit_run_dir",
            resolver.cortexkit_run_dir(),
            data.map(|p| p.join("cortexkit").join("run")),
        ),
        (
            "cortexkit_state_dir",
            resolver.cortexkit_state_dir(),
            state.map(|p| p.join("cortexkit")),
        ),
        (
            "cortexkit_cache_dir",
            resolver.cortexkit_cache_dir(),
            cache.map(|p| p.join("cortexkit")),
        ),
    ];
    for (name, result, expected) in results {
        match expected {
            Some(path) => {
                let actual =
                    result.unwrap_or_else(|error| panic!("{name}: {error}; inputs {:?}", env.vars));
                assert!(actual.is_absolute(), "{name}: {actual:?}");
                assert_eq!(actual, path, "{name}: inputs {:?}", env.vars);
            }
            None => assert!(
                matches!(result, Err(PathsError::Unresolved { .. })),
                "{name}: {result:?}; inputs {:?}",
                env.vars
            ),
        }
    }
    // Discovery intentionally reads HOME rather than the platform's data home.
    let expected = env
        .vars
        .get("HOME")
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute());
    let mut candidates = Vec::new();
    if let Some(home) = expected {
        candidates.push(
            home.join(".local")
                .join("share")
                .join("cortexkit")
                .join("run")
                .join("subc-connection.json"),
        );
    }
    candidates.push(env.temp.join("subc-user.connection.json"));
    assert_eq!(
        resolver.connection_file_candidates(None, "user").unwrap(),
        candidates
    );
}

#[cfg(unix)]
mod unix {
    use super::*;

    #[test]
    fn unix_directory_table() {
        for home_value in VALUES {
            for xdg_values in xdg_inputs() {
                let mut env = TestEnv::default();
                input(&mut env, "HOME", home_value, &absolute("home"));
                // Windows variables must not influence Unix resolution.
                env.set("USERPROFILE", absolute("profile"));
                env.set("APPDATA", absolute("roaming"));
                env.set("LOCALAPPDATA", absolute("local"));
                for (key, value) in XDG.into_iter().zip(xdg_values) {
                    input(&mut env, key, value, &absolute(key));
                }
                let home = matches!(home_value, Value::Set).then(|| absolute("home"));
                let tails = [
                    vec![".local", "share"],
                    vec![".config"],
                    vec![".local", "state"],
                    vec![".cache"],
                ];
                let bases = std::array::from_fn(|i| {
                    let fallback = home.clone().map(|mut p| {
                        for part in &tails[i] {
                            p.push(part);
                        }
                        p
                    });
                    expected_xdg(xdg_values[i], absolute(XDG[i]), fallback)
                });
                check_directories(&env, Platform::Unix, home, bases);
            }
        }
    }

    #[test]
    fn non_unicode_home_is_preserved() {
        use std::os::unix::ffi::OsStringExt;
        let mut env = TestEnv::default();
        let home = OsString::from_vec(vec![b'/', b'h', 0xff]);
        env.set("HOME", home.clone());
        assert_eq!(
            with_env(&env).data_home().unwrap(),
            PathBuf::from(home).join(".local").join("share")
        );
    }
}

#[cfg(windows)]
mod windows {
    use super::*;

    #[test]
    fn windows_directory_table() {
        for profile_value in VALUES {
            for home_value in VALUES {
                for roaming_value in VALUES {
                    for local_value in VALUES {
                        for xdg_values in xdg_inputs() {
                            let mut env = TestEnv::default();
                            input(&mut env, "USERPROFILE", profile_value, &absolute("profile"));
                            input(&mut env, "APPDATA", roaming_value, &absolute("roaming"));
                            input(&mut env, "LOCALAPPDATA", local_value, &absolute("local"));
                            input(&mut env, "HOME", home_value, &absolute("git-bash-home"));
                            for (key, value) in XDG.into_iter().zip(xdg_values) {
                                input(&mut env, key, value, &absolute(key));
                            }
                            let profile =
                                matches!(profile_value, Value::Set).then(|| absolute("profile"));
                            let local = expected_xdg(
                                local_value,
                                absolute("local"),
                                profile.as_ref().map(|p| p.join("AppData").join("Local")),
                            );
                            let roaming = expected_xdg(
                                roaming_value,
                                absolute("roaming"),
                                profile.as_ref().map(|p| p.join("AppData").join("Roaming")),
                            );
                            let bases = std::array::from_fn(|i| {
                                expected_xdg(
                                    xdg_values[i],
                                    absolute(XDG[i]),
                                    if i == 1 {
                                        roaming.clone()
                                    } else {
                                        local.clone()
                                    },
                                )
                            });
                            check_directories(
                                &env,
                                Platform::Windows,
                                profile.or_else(|| {
                                    matches!(home_value, Value::Set)
                                        .then(|| absolute("git-bash-home"))
                                }),
                                bases,
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn windows_home_drive_and_path_table() {
        for profile_value in VALUES {
            for home_value in VALUES {
                for drive_value in VALUES {
                    for path_value in VALUES {
                        let mut env = TestEnv::default();
                        input(&mut env, "USERPROFILE", profile_value, &absolute("profile"));
                        input(&mut env, "HOME", home_value, &absolute("git-bash-home"));
                        match drive_value {
                            Value::Set => env.set("HOMEDRIVE", "C:"),
                            Value::Empty => env.set("HOMEDRIVE", ""),
                            Value::Relative => env.set("HOMEDRIVE", "relative"),
                            Value::Unset => {}
                        }
                        match path_value {
                            Value::Set => env.set("HOMEPATH", r"\Users\user"),
                            Value::Empty => env.set("HOMEPATH", ""),
                            Value::Relative => env.set("HOMEPATH", "relative"),
                            Value::Unset => {}
                        }
                        let expected = if matches!(profile_value, Value::Set) {
                            Some(absolute("profile"))
                        } else if matches!((drive_value, path_value), (Value::Set, Value::Set)) {
                            Some(PathBuf::from(r"C:\Users\user"))
                        } else {
                            matches!(home_value, Value::Set).then(|| absolute("git-bash-home"))
                        };
                        assert_eq!(
                            with_env(&env).home().ok(),
                            expected,
                            "{profile_value:?}, {drive_value:?}, {path_value:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn windows_drive_relative_explicit_candidate_is_never_returned() {
        let env = TestEnv::default();
        let result =
            with_env(&env).connection_file_candidates(Some(Path::new("D:conn.json")), "user");
        assert!(matches!(result, Err(PathsError::RelativePath { .. })));
    }
}

fn windows_env() -> TestEnv {
    let mut env = TestEnv::default();
    env.set("USERPROFILE", absolute("profile"));
    env.set("HOME", absolute("git-bash-home"));
    env.set("APPDATA", absolute("roaming"));
    env.set("LOCALAPPDATA", absolute("local"));
    env
}

// The policy can be exercised on Unix too; the Windows-only table additionally
// checks native drive prefixes and rooted path semantics.
#[test]
fn windows_policy_uses_local_data_and_roaming_config() {
    let env = windows_env();
    let r = env.resolver(Platform::Windows);
    assert_eq!(r.data_home().unwrap(), absolute("local"));
    assert_eq!(r.state_home().unwrap(), absolute("local"));
    assert_eq!(r.cache_home().unwrap(), absolute("local"));
    assert_eq!(r.config_home().unwrap(), absolute("roaming"));
}

#[test]
fn windows_legacy_only_preserved_without_local_data() {
    for legacy_exists in [false, true] {
        for local_exists in [false, true] {
            let mut env = windows_env();
            let legacy = absolute("roaming").join("cortexkit");
            let local = absolute("local").join("cortexkit");
            if legacy_exists {
                env.existing.insert(legacy.clone());
            }
            if local_exists {
                env.existing.insert(local.clone());
            }
            let expected = if legacy_exists && !local_exists {
                legacy
            } else {
                local
            };
            let r = env.resolver(Platform::Windows);
            assert_eq!(r.cortexkit_data_dir().unwrap(), expected);
            assert_eq!(r.cortexkit_run_dir().unwrap(), expected.join("run"));
            // Only CortexKit data and run can retain an existing roaming install;
            // state and cache stay local even when data retains its old location.
            assert_eq!(r.data_home().unwrap(), absolute("local"));
            assert_eq!(
                r.cortexkit_state_dir().unwrap(),
                absolute("local").join("cortexkit")
            );
            assert_eq!(
                r.cortexkit_cache_dir().unwrap(),
                absolute("local").join("cortexkit")
            );
        }
    }
}

#[test]
fn windows_xdg_override_wins_over_legacy() {
    let mut env = windows_env();
    env.existing.insert(absolute("roaming").join("cortexkit"));
    env.set("XDG_DATA_HOME", absolute("xdg"));
    assert_eq!(
        env.resolver(Platform::Windows)
            .cortexkit_data_dir()
            .unwrap(),
        absolute("xdg").join("cortexkit")
    );
}

#[test]
fn windows_profile_appdata_fallbacks() {
    for value in [Value::Empty, Value::Relative, Value::Unset] {
        let mut env = TestEnv::default();
        env.set("USERPROFILE", absolute("profile"));
        input(&mut env, "APPDATA", value, &absolute("roaming"));
        input(&mut env, "LOCALAPPDATA", value, &absolute("local"));
        let r = env.resolver(Platform::Windows);
        assert_eq!(
            r.data_home().unwrap(),
            absolute("profile").join("AppData").join("Local")
        );
        assert_eq!(
            r.config_home().unwrap(),
            absolute("profile").join("AppData").join("Roaming")
        );
        env.existing.insert(
            absolute("profile")
                .join("AppData")
                .join("Roaming")
                .join("cortexkit"),
        );
        assert_eq!(
            env.resolver(Platform::Windows)
                .cortexkit_data_dir()
                .unwrap(),
            absolute("profile")
                .join("AppData")
                .join("Roaming")
                .join("cortexkit")
        );
    }
}

#[test]
fn windows_home_prefers_profile_over_home_then_falls_back_to_home() {
    let mut env = windows_env();
    assert_eq!(
        env.resolver(Platform::Windows).home().unwrap(),
        absolute("profile")
    );
    for value in [Value::Empty, Value::Relative, Value::Unset] {
        env.vars.remove("USERPROFILE");
        input(&mut env, "USERPROFILE", value, &absolute("profile"));
        assert_eq!(
            env.resolver(Platform::Windows).home().unwrap(),
            absolute("git-bash-home")
        );
    }
}

#[test]
fn xdg_data_layout_uses_home_on_every_platform() {
    for platform in [Platform::Unix, Platform::Windows] {
        for value in VALUES {
            let mut env = windows_env();
            input(&mut env, "XDG_DATA_HOME", value, &absolute("xdg"));
            let home = if matches!(platform, Platform::Unix) {
                absolute("git-bash-home")
            } else {
                absolute("profile")
            };
            let expected = expected_xdg(
                value,
                absolute("xdg"),
                Some(home.join(".local").join("share")),
            )
            .unwrap();
            assert_eq!(env.resolver(platform).xdg_data_home().unwrap(), expected);
        }
    }
    let mut env = TestEnv::default();
    env.set("LOCALAPPDATA", absolute("local"));
    assert!(env.resolver(Platform::Windows).xdg_data_home().is_err());
}

#[test]
fn consumer_absolute_path_variable_table() {
    for value in VALUES {
        let mut env = TestEnv::default();
        input(&mut env, "CODEX_HOME", value, &absolute("codex"));
        match value {
            Value::Set => assert_eq!(
                env_absolute_path(&env, "CODEX_HOME").unwrap(),
                Some(absolute("codex"))
            ),
            Value::Empty | Value::Unset => {
                assert_eq!(env_absolute_path(&env, "CODEX_HOME").unwrap(), None)
            }
            Value::Relative => {
                let error = env_absolute_path(&env, "CODEX_HOME").unwrap_err();
                assert!(
                    matches!(&error, PathsError::RelativePath { source, .. } if source == "CODEX_HOME")
                );
                assert!(error.to_string().contains("CODEX_HOME"));
            }
        }
    }
    let env = TestEnv::default();
    let source: &dyn EnvSource = &env;
    assert_eq!(env_absolute_path(source, "CODEX_HOME").unwrap(), None);
    assert!(with_env(source).home().is_err());
}

#[test]
fn relative_xdg_values_are_ignored() {
    for platform in [Platform::Unix, Platform::Windows] {
        let mut env = windows_env();
        for key in XDG {
            env.set(key, "relative");
        }
        let r = env.resolver(platform);
        let expected = if matches!(platform, Platform::Unix) {
            let home = absolute("git-bash-home");
            [
                home.join(".local").join("share"),
                home.join(".config"),
                home.join(".local").join("state"),
                home.join(".cache"),
            ]
        } else {
            [
                absolute("local"),
                absolute("roaming"),
                absolute("local"),
                absolute("local"),
            ]
        };
        let actual = [
            r.data_home().unwrap(),
            r.config_home().unwrap(),
            r.state_home().unwrap(),
            r.cache_home().unwrap(),
        ];
        assert_eq!(actual, expected);
        assert!(actual.iter().all(|p| p.is_absolute()));
    }
}

#[test]
fn missing_directories_report_all_tried_variables() {
    let env = TestEnv::default();
    for platform in [Platform::Unix, Platform::Windows] {
        let r = env.resolver(platform);
        let (home_vars, app_vars) = if matches!(platform, Platform::Unix) {
            (vec!["HOME"], vec!["HOME"])
        } else {
            (
                vec!["USERPROFILE", "HOMEDRIVE", "HOMEPATH", "HOME"],
                vec!["LOCALAPPDATA", "USERPROFILE"],
            )
        };
        let home_error = r.home().unwrap_err().to_string();
        for key in home_vars {
            assert!(home_error.contains(key), "{home_error}");
        }
        for (xdg, error) in [
            ("XDG_DATA_HOME", r.data_home().unwrap_err()),
            ("XDG_CONFIG_HOME", r.config_home().unwrap_err()),
            ("XDG_STATE_HOME", r.state_home().unwrap_err()),
            ("XDG_CACHE_HOME", r.cache_home().unwrap_err()),
        ] {
            let message = error.to_string();
            assert!(message.contains(xdg));
            for key in &app_vars {
                let key = if xdg == "XDG_CONFIG_HOME" && *key == "LOCALAPPDATA" {
                    "APPDATA"
                } else {
                    key
                };
                assert!(message.contains(key), "{message}");
            }
        }
    }
}

// A copy of how the subc daemon resolves its run directory before it adopts this
// crate: XDG_DATA_HOME, then on Windows APPDATA or USERPROFILE\AppData\Roaming,
// then HOME/.local/share, plus cortexkit/run, refusing a relative result. It
// honours a relative XDG value, which this crate ignores. The test below proves
// the two agree wherever their policies are meant to.
fn daemon_run_reference(env: &TestEnv, platform: Platform) -> Option<PathBuf> {
    let non_empty = |key: &str| {
        env.var_os(key)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let data = non_empty("XDG_DATA_HOME")
        .or_else(|| {
            if matches!(platform, Platform::Windows) {
                non_empty("APPDATA")
                    .or_else(|| non_empty("USERPROFILE").map(|p| p.join("AppData").join("Roaming")))
            } else {
                None
            }
        })
        .or_else(|| non_empty("HOME").map(|p| p.join(".local").join("share")))
        .unwrap_or_else(|| PathBuf::from(".local").join("share"));
    data.is_absolute()
        .then(|| data.join("cortexkit").join("run"))
}

#[test]
fn run_dir_matches_daemon_ladder_when_policies_agree() {
    for platform in [Platform::Unix, Platform::Windows] {
        for value in [Value::Set, Value::Empty, Value::Unset] {
            let mut env = windows_env();
            // The daemon's copy uses roaming APPDATA for data and this crate uses
            // LOCALAPPDATA, so point both at one root to compare them. Relative
            // XDG values are left out because the two deliberately differ there.
            env.set("LOCALAPPDATA", absolute("roaming"));
            input(&mut env, "XDG_DATA_HOME", value, &absolute("xdg"));
            assert_eq!(
                env.resolver(platform).cortexkit_run_dir().ok(),
                daemon_run_reference(&env, platform)
            );
        }
    }
    let env = TestEnv::default();
    assert_eq!(
        env.resolver(Platform::Unix).cortexkit_run_dir().ok(),
        daemon_run_reference(&env, Platform::Unix)
    );
    let mut env = TestEnv::default();
    env.set("USERPROFILE", absolute("profile"));
    env.set(
        "LOCALAPPDATA",
        absolute("profile").join("AppData").join("Roaming"),
    );
    assert_eq!(
        env.resolver(Platform::Windows).cortexkit_run_dir().ok(),
        daemon_run_reference(&env, Platform::Windows)
    );
}

#[test]
fn windows_run_dir_intentionally_differs_from_old_roaming_daemon() {
    let env = windows_env();
    let actual = env.resolver(Platform::Windows).cortexkit_run_dir().unwrap();
    assert_eq!(actual, absolute("local").join("cortexkit").join("run"));
    assert_eq!(
        daemon_run_reference(&env, Platform::Windows).unwrap(),
        absolute("roaming").join("cortexkit").join("run")
    );
    assert_ne!(Some(actual), daemon_run_reference(&env, Platform::Windows));
}

#[test]
fn connection_candidates_match_transport_order_and_exclusivity() {
    for runtime in VALUES {
        for home in VALUES {
            for override_value in VALUES {
                let mut env = TestEnv::default();
                input(&mut env, "XDG_RUNTIME_DIR", runtime, &absolute("runtime"));
                input(&mut env, "HOME", home, &absolute("home"));
                input(
                    &mut env,
                    "SUBC_CONNECTION_FILE",
                    override_value,
                    &absolute("override.json"),
                );
                let r = with_env(&env);
                let result = r.connection_file_candidates(None, "user");
                if matches!(override_value, Value::Relative) {
                    assert!(
                        matches!(result, Err(PathsError::RelativePath { source, .. }) if source == "SUBC_CONNECTION_FILE")
                    );
                } else {
                    let expected = if matches!(override_value, Value::Set) {
                        vec![absolute("override.json")]
                    } else {
                        let mut paths = Vec::new();
                        if matches!(runtime, Value::Set) {
                            paths.push(absolute("runtime").join("subc-connection.json"));
                        }
                        if matches!(home, Value::Set) {
                            paths.push(
                                absolute("home")
                                    .join(".local")
                                    .join("share")
                                    .join("cortexkit")
                                    .join("run")
                                    .join("subc-connection.json"),
                            );
                        }
                        paths.push(env.temp.join("subc-user.connection.json"));
                        paths
                    };
                    let actual = result.unwrap();
                    assert_eq!(actual, expected);
                    assert!(actual.iter().all(|p| p.is_absolute()));
                }
                // An explicit argument always wins, even over an invalid env override.
                assert_eq!(
                    r.connection_file_candidates(Some(&absolute("explicit.json")), "user")
                        .unwrap(),
                    vec![absolute("explicit.json")]
                );
                assert_eq!(
                    r.connection_file_candidates(Some(Path::new("conn.json")), "user")
                        .unwrap(),
                    vec![env.cwd.join("conn.json")]
                );
            }
        }
    }
}

#[test]
fn connection_candidates_reject_relative_bases_and_invalid_tokens() {
    let mut env = TestEnv {
        temp: PathBuf::from("relative"),
        ..TestEnv::default()
    };
    assert!(
        matches!(with_env(&env).connection_file_candidates(None, "user"), Err(PathsError::RelativePath { source, .. }) if source == "temporary directory")
    );
    env.temp = absolute("temp");
    for token in ["", "a/b", r"a\b"] {
        assert!(matches!(
            with_env(&env).connection_file_candidates(None, token),
            Err(PathsError::InvalidUserToken)
        ));
    }
    env.cwd = PathBuf::from("relative");
    assert!(
        matches!(with_env(&env).connection_file_candidates(Some(Path::new("conn.json")), "user"), Err(PathsError::RelativePath { source, .. }) if source == "current directory")
    );
    env.cwd_error = true;
    let error = with_env(&env)
        .connection_file_candidates(Some(Path::new("conn.json")), "user")
        .unwrap_err();
    assert!(error.source().is_some());
    assert!(matches!(error, PathsError::CurrentDir(_)));
}

#[test]
fn xdg_overrides_are_independent() {
    for platform in [Platform::Unix, Platform::Windows] {
        for key in XDG {
            let mut env = windows_env();
            env.set(key, absolute("override"));
            let r = env.resolver(platform);
            let home = absolute("git-bash-home");
            let fallback = if matches!(platform, Platform::Windows) {
                [
                    absolute("local"),
                    absolute("roaming"),
                    absolute("local"),
                    absolute("local"),
                ]
            } else {
                [
                    home.join(".local").join("share"),
                    home.join(".config"),
                    home.join(".local").join("state"),
                    home.join(".cache"),
                ]
            };
            let actual = [
                r.data_home().unwrap(),
                r.config_home().unwrap(),
                r.state_home().unwrap(),
                r.cache_home().unwrap(),
            ];
            for i in 0..4 {
                assert_eq!(
                    actual[i],
                    if key == XDG[i] {
                        absolute("override")
                    } else {
                        fallback[i].clone()
                    },
                    "{key}"
                );
            }
        }
    }
}

#[test]
fn windows_home_drive_pair_precedes_home() {
    let mut env = TestEnv::default();
    // An absolute drive base makes the fallback branch testable on every host;
    // the Windows-only table separately supplies native C: and rooted HOMEPATH.
    env.set("HOMEDRIVE", absolute("drive"));
    env.set("HOMEPATH", "user");
    env.set("HOME", absolute("git-bash-home"));
    assert_eq!(
        env.resolver(Platform::Windows).home().unwrap(),
        absolute("drive").join("user")
    );
}

#[test]
fn windows_existing_legacy_data_survives_missing_local_home() {
    let mut env = TestEnv::default();
    env.set("APPDATA", absolute("roaming"));
    env.existing.insert(absolute("roaming").join("cortexkit"));
    let r = env.resolver(Platform::Windows);
    assert!(r.data_home().is_err());
    assert_eq!(
        r.cortexkit_data_dir().unwrap(),
        absolute("roaming").join("cortexkit")
    );
}
