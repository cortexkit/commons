//! Tiers: where layers come from, and how much they are trusted.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    digest::normalize_newlines,
    ids::{check_bare, check_relative_path, check_role, check_segment},
    PersonaError, SourceRef,
};

/// The largest file a layer may contain, in bytes.
pub const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// The manifest's logical path inside a layer.
pub(crate) const MANIFEST: &str = "PERSONA.md";

/// How far a tier's files are trusted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustClass {
    /// The operator's own files (the embedded base, the user and workspace
    /// tiers, an installed catalog folder): applied as found.
    Operator,
    /// Files that arrive with a repository: applied only when the operator
    /// approved that layer's digest for that project.
    Repository,
}

/// Role bundles compiled into the module, keyed by role and logical path.
///
/// A module typically fills this with `include_str!` of its base bundles.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EmbeddedTier {
    roles: BTreeMap<String, BTreeMap<String, String>>,
}

impl EmbeddedTier {
    /// An empty set of bundles.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one file of a role's bundle. `logical_path` is `PERSONA.md` or
    /// `fragments/<bare-id>.md`.
    pub fn insert(
        &mut self,
        role: &str,
        logical_path: &str,
        contents: impl Into<String>,
    ) -> Result<(), PersonaError> {
        check_role(role)?;
        if logical_path != MANIFEST && fragment_file_id(logical_path).is_none() {
            return Err(PersonaError::InvalidId {
                what: "bundle file path",
                value: logical_path.to_string(),
                reason: "must be `PERSONA.md` or `fragments/<bare-id>.md`",
            });
        }
        self.roles
            .entry(role.to_string())
            .or_default()
            .insert(logical_path.to_string(), contents.into());
        Ok(())
    }

    /// Builder form of [`Self::insert`].
    pub fn with_file(
        mut self,
        role: &str,
        logical_path: &str,
        contents: impl Into<String>,
    ) -> Result<Self, PersonaError> {
        self.insert(role, logical_path, contents)?;
        Ok(self)
    }
}

/// The bare fragment id of a `fragments/<id>.md` logical path.
pub(crate) fn fragment_file_id(logical_path: &str) -> Option<&str> {
    let id = logical_path
        .strip_prefix("fragments/")?
        .strip_suffix(".md")?;
    check_bare("fragment id", id).ok().map(|()| id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TierSource {
    Directory(PathBuf),
    Embedded(EmbeddedTier),
}

/// One rung of the ladder a module passes to [`crate::PersonaEngine::load`],
/// least to most specific.
///
/// A role's layer in this tier lives at `<root>/<roles_path>/<role>/`. The
/// tier-relative path `<roles_path>/<role>` is what an approval names, so a
/// moved project keeps its approvals.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tier {
    name: String,
    trust: TrustClass,
    roles_path: String,
    source: TierSource,
}

impl Tier {
    /// A tier on disk. `name` is a bare id unique in the ladder; `roles_path`
    /// is `/`-separated and relative to `root`, for example
    /// `.cortexkit/demo/roles`.
    pub fn directory(
        name: &str,
        trust: TrustClass,
        root: impl Into<PathBuf>,
        roles_path: &str,
    ) -> Result<Self, PersonaError> {
        check_bare("tier name", name)?;
        check_relative_path("tier roles path", roles_path)?;
        Ok(Tier {
            name: name.to_string(),
            trust,
            roles_path: roles_path.to_string(),
            source: TierSource::Directory(root.into()),
        })
    }

    /// A tier compiled into the module. Its tier-relative paths are
    /// `roles/<role>`.
    pub fn embedded(
        name: &str,
        trust: TrustClass,
        bundles: EmbeddedTier,
    ) -> Result<Self, PersonaError> {
        check_bare("tier name", name)?;
        Ok(Tier {
            name: name.to_string(),
            trust,
            roles_path: "roles".to_string(),
            source: TierSource::Embedded(bundles),
        })
    }

    /// The tier's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The tier's trust class.
    pub fn trust(&self) -> TrustClass {
        self.trust
    }

    /// The tier root as given, or `None` for an embedded tier.
    pub fn root(&self) -> Option<&Path> {
        match &self.source {
            TierSource::Directory(root) => Some(root),
            TierSource::Embedded(_) => None,
        }
    }

    /// The tier-relative path of `role`'s layer, `<roles_path>/<role>`.
    pub fn relative_path(&self, role: &str) -> String {
        format!("{}/{}", self.roles_path, role)
    }

    pub(crate) fn file_ref(&self, role: &str, logical_path: &str) -> SourceRef {
        SourceRef::File {
            tier: self.name.clone(),
            path: format!("{}/{}", self.relative_path(role), logical_path),
        }
    }

    /// Read `role`'s layer: `PERSONA.md` and every `fragments/<bare-id>.md`,
    /// LF-normalised. `None` when the tier has no `PERSONA.md` for the role.
    /// Other files in the directory are not part of the layer.
    pub(crate) fn read_layer(
        &self,
        role: &str,
    ) -> Result<Option<BTreeMap<String, String>>, PersonaError> {
        match &self.source {
            TierSource::Embedded(bundles) => {
                let Some(files) = bundles.roles.get(role) else {
                    return Ok(None);
                };
                if !files.contains_key(MANIFEST) {
                    return Ok(None);
                }
                let mut out = BTreeMap::new();
                for (path, text) in files {
                    let file = self.file_ref(role, path);
                    if text.len() as u64 > MAX_FILE_BYTES {
                        return Err(PersonaError::FileTooLarge {
                            file,
                            limit: MAX_FILE_BYTES,
                        });
                    }
                    out.insert(path.clone(), normalize_newlines(text).into_owned());
                }
                Ok(Some(out))
            }
            TierSource::Directory(root) => {
                let repository = self.trust == TrustClass::Repository;
                // A repository tier's root may itself be reached through a
                // link (a checkout under a linked folder), so it is resolved
                // once; below it, every component is checked for links.
                let base = if repository {
                    match fs::canonicalize(root) {
                        Ok(path) => path,
                        Err(error) if is_absent(&error) => return Ok(None),
                        Err(error) => {
                            return Err(PersonaError::Io {
                                path: root.clone(),
                                error,
                            })
                        }
                    }
                } else {
                    root.clone()
                };
                let mut dir = base.clone();
                let mut relative = String::new();
                for segment in self.roles_path.split('/').chain(std::iter::once(role)) {
                    dir.push(segment);
                    if !relative.is_empty() {
                        relative.push('/');
                    }
                    relative.push_str(segment);
                    if repository && !self.plain_directory(&dir, &relative)? {
                        return Ok(None);
                    }
                }
                let Some(manifest) = self.read_file(&base, &dir.join(MANIFEST), role, MANIFEST)?
                else {
                    return Ok(None);
                };
                let mut out = BTreeMap::from([(MANIFEST.to_string(), manifest)]);
                let fragments_dir = dir.join("fragments");
                if repository
                    && !self.plain_directory(&fragments_dir, &format!("{relative}/fragments"))?
                {
                    return Ok(Some(out));
                }
                let entries = match fs::read_dir(&fragments_dir) {
                    Ok(entries) => entries,
                    Err(error) if is_absent(&error) => return Ok(Some(out)),
                    Err(error) => {
                        return Err(PersonaError::Io {
                            path: fragments_dir,
                            error,
                        })
                    }
                };
                for entry in entries {
                    let entry = entry.map_err(|error| PersonaError::Io {
                        path: fragments_dir.clone(),
                        error,
                    })?;
                    // Only `<bare-id>.md` names are fragments. Anything else,
                    // including a file named like a qualified id, is not part
                    // of the layer: qualified ids come only from the module.
                    let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                        continue;
                    };
                    let logical = format!("fragments/{name}");
                    if fragment_file_id(&logical).is_none() {
                        continue;
                    }
                    if let Some(text) = self.read_file(&base, &entry.path(), role, &logical)? {
                        out.insert(logical, text);
                    }
                }
                Ok(Some(out))
            }
        }
    }

    /// For a repository tier: whether `path` (tier-relative `relative`) is a
    /// real directory. Absent, or a file where a directory belongs, means the
    /// layer is absent; a symbolic link or junction is refused. On Windows,
    /// std reports a junction as a link, because `is_symlink` is true for
    /// every name-surrogate reparse point.
    fn plain_directory(&self, path: &Path, relative: &str) -> Result<bool, PersonaError> {
        match fs::symlink_metadata(path) {
            Ok(m) if m.file_type().is_symlink() => Err(PersonaError::LinkInRepositoryTier {
                component: SourceRef::File {
                    tier: self.name.clone(),
                    path: relative.to_string(),
                },
            }),
            Ok(m) => Ok(m.is_dir()),
            Err(error) if is_absent(&error) => Ok(false),
            Err(error) => Err(PersonaError::Io {
                path: path.to_path_buf(),
                error,
            }),
        }
    }

    /// Read one layer file. For a repository tier, `base` is the resolved
    /// tier root and every directory above the file has already been checked;
    /// here the file itself must not be a link and must resolve inside `base`.
    fn read_file(
        &self,
        base: &Path,
        path: &Path,
        role: &str,
        logical: &str,
    ) -> Result<Option<String>, PersonaError> {
        // Repository files must not be links: a link could pull any file the
        // process can read into the persona. Operator tiers may be symlinked
        // by dotfile managers, so they follow links.
        let repository = self.trust == TrustClass::Repository;
        let metadata = if repository {
            fs::symlink_metadata(path)
        } else {
            fs::metadata(path)
        };
        let metadata = match metadata {
            Ok(m) => m,
            Err(error) if is_absent(&error) => return Ok(None),
            Err(error) => {
                return Err(PersonaError::Io {
                    path: path.to_path_buf(),
                    error,
                })
            }
        };
        let file = self.file_ref(role, logical);
        if repository && metadata.file_type().is_symlink() {
            return Err(PersonaError::LinkInRepositoryTier { component: file });
        }
        if !metadata.is_file() {
            return Err(PersonaError::NotRegularFile { file });
        }
        if repository {
            // With no link below the root this always holds; it is checked
            // anyway so that a later change to the path rules cannot quietly
            // read outside the repository.
            let resolved = fs::canonicalize(path).map_err(|error| PersonaError::Io {
                path: path.to_path_buf(),
                error,
            })?;
            if !resolved.starts_with(base) {
                return Err(PersonaError::OutsideTierRoot { file });
            }
        }
        if metadata.len() > MAX_FILE_BYTES {
            return Err(PersonaError::FileTooLarge {
                file,
                limit: MAX_FILE_BYTES,
            });
        }
        let bytes = fs::read(path).map_err(|error| PersonaError::Io {
            path: path.to_path_buf(),
            error,
        })?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(PersonaError::FileTooLarge {
                file,
                limit: MAX_FILE_BYTES,
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| PersonaError::NotUtf8 { file })?;
        Ok(Some(normalize_newlines(&text).into_owned()))
    }
}

/// A path below a regular file fails with "not a directory" on Unix but
/// "not found" on Windows; both mean the layer is absent.
fn is_absent(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
    )
}

/// The platform config directory, from
/// `cortexkit_store_types::resolve_config_home`. Lookup order: a non-empty
/// `XDG_CONFIG_HOME`; on Windows, then `APPDATA`, then
/// `USERPROFILE\AppData\Roaming`; then `HOME/.config`; when none of those is
/// set the resolver answers a bare `.config`. Empty values count as unset. A
/// result that is not absolute (that bare `.config`, or a relative
/// `XDG_CONFIG_HOME`) is refused, because it would resolve against whatever
/// the current directory happens to be. A change to that crate's lookup
/// changes the default ladder here.
pub fn config_home() -> Result<PathBuf, PersonaError> {
    absolute_config_home(cortexkit_store_types::resolve_config_home())
}

/// Accept the resolver's answer only when it is absolute.
fn absolute_config_home(resolved: String) -> Result<PathBuf, PersonaError> {
    let path = PathBuf::from(&resolved);
    if !path.is_absolute() {
        return Err(PersonaError::ConfigHomeNotAbsolute { resolved });
    }
    Ok(path)
}

/// The default ladder of SPEC.md section 4:
///
/// 1. `base`: the module's embedded bundles (`operator`);
/// 2. `user`: `<config>/cortexkit/<module>/roles/<role>/` (`operator`);
/// 3. `workspace`: `<config>/cortexkit/<module>/workspaces/<ws>/roles/<role>/`
///    (`operator`), when a workspace is set;
/// 4. `project`: `<project>/.cortexkit/<module>/roles/<role>/` (`repository`),
///    when a project root is set.
#[derive(Clone, Debug)]
pub struct DefaultLadder {
    module: String,
    base: EmbeddedTier,
    config_home: PathBuf,
    workspace: Option<String>,
    project_root: Option<PathBuf>,
}

impl DefaultLadder {
    /// The ladder under the platform config directory from [`config_home`].
    pub fn new(module: &str, base: EmbeddedTier) -> Result<Self, PersonaError> {
        Self::with_config_home(module, base, config_home()?)
    }

    /// The ladder under an explicit config directory, for tests and for a
    /// module that already resolved it.
    pub fn with_config_home(
        module: &str,
        base: EmbeddedTier,
        config_home: impl Into<PathBuf>,
    ) -> Result<Self, PersonaError> {
        check_bare("module id", module)?;
        Ok(DefaultLadder {
            module: module.to_string(),
            base,
            config_home: config_home.into(),
            workspace: None,
            project_root: None,
        })
    }

    /// Add the workspace tier for workspace `id`.
    pub fn workspace(mut self, id: &str) -> Result<Self, PersonaError> {
        check_segment("workspace id", id)?;
        self.workspace = Some(id.to_string());
        Ok(self)
    }

    /// Add the project tier under the project's root directory.
    pub fn project(mut self, root: impl Into<PathBuf>) -> Self {
        self.project_root = Some(root.into());
        self
    }

    /// The tiers, least to most specific.
    pub fn tiers(&self) -> Vec<Tier> {
        let module = &self.module;
        // Every name and path below is built from checked ids, so these
        // constructors cannot fail.
        let mut tiers = vec![
            Tier::embedded("base", TrustClass::Operator, self.base.clone()).expect("fixed name"),
            Tier::directory(
                "user",
                TrustClass::Operator,
                self.config_home.clone(),
                &format!("cortexkit/{module}/roles"),
            )
            .expect("checked ids"),
        ];
        if let Some(ws) = &self.workspace {
            tiers.push(
                Tier::directory(
                    "workspace",
                    TrustClass::Operator,
                    self.config_home.clone(),
                    &format!("cortexkit/{module}/workspaces/{ws}/roles"),
                )
                .expect("checked ids"),
            );
        }
        if let Some(root) = &self.project_root {
            tiers.push(
                Tier::directory(
                    "project",
                    TrustClass::Repository,
                    root.clone(),
                    &format!(".cortexkit/{module}/roles"),
                )
                .expect("checked ids"),
            );
        }
        tiers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_config_home_is_refused() {
        // The resolver falls back to a bare `.config` when no variable is set.
        assert!(matches!(
            absolute_config_home(".config".into()),
            Err(PersonaError::ConfigHomeNotAbsolute { resolved }) if resolved == ".config"
        ));
        let absolute = std::env::temp_dir().join("config");
        assert_eq!(
            absolute_config_home(absolute.to_string_lossy().into_owned()).unwrap(),
            absolute
        );
    }
}
