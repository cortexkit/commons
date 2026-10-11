# Changelog

## 0.1.2

- Add `dirs` and crate-root re-exports for absolute home, data, config, state,
  cache, and CortexKit-specific directories. Public `EnvSource`, `SystemEnv`,
  and `with_env` support hermetic consumer tests without environment mutation.
- Use XDG layout on Unix, including macOS. Empty and relative XDG overrides are
  ignored; missing absolute fallbacks return `PathsError` naming the variables
  tried. This intentionally differs from older daemon resolution, which honored
  relative XDG overrides and could return a relative final fallback.
- Use local Windows application data for data, state, and cache; config uses
  roaming application data. Preserve an existing roaming `cortexkit` data
  directory only when no local install or absolute XDG override exists. No
  automatic migration occurs. The run directory follows the selected data
  directory; new Windows installs intentionally differ from older roaming-data
  daemons until those daemons adopt this resolver.
- Resolve Windows home in order: `USERPROFILE`, `HOMEDRIVE` + `HOMEPATH`, then
  `HOME`, so Git Bash's `HOME` does not override the native profile.
- Add `env_absolute_path` for consumers' own environment variables, with a named
  error for relative values. Add `xdg_data_home` for applications using XDG layout
  on every OS, distinct from the Windows-local `data_home` policy.
- Add ordered connection-file discovery with exclusive explicit/environment
  overrides and a caller-provided per-user token. Relative explicit arguments
  are resolved against the current directory; relative environment overrides
  are errors, and relative runtime/home candidates are skipped. Discovery keeps
  the transport's HOME-based fallback, rather than substituting the new data
  directory policy.
- Leave `ProjectRootId` and its canonicalization behavior unchanged.
