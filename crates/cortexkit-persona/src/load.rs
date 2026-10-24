//! Loading: read every tier, decide approvals, merge the applied layers.

use std::{
    collections::{BTreeMap, BTreeSet},
    marker::PhantomData,
    sync::Arc,
};

use serde::{de::DeserializeOwned, Serialize};

use crate::{
    approval::{ApprovalSet, ApprovalState},
    digest::layer_digest,
    extension::{AnyBox, Extension, Typed},
    ids::{check_bare, check_role},
    manifest::{parse_persona, LayerManifest, Manifest, Registry, CORE_FIELDS, REQUIRED_FIELDS},
    report::{ParentReport, TierReport},
    tier::{fragment_file_id, MANIFEST},
    Digest, PersonaError, ProjectKey, SourceRef, Tier,
};

/// One module's persona engine: its module id and its registered extensions.
pub struct PersonaEngine {
    module: String,
    registry: Registry,
}

/// What to load: a role, the module's tier ladder, and the approvals and
/// project identity that decide `repository` layers.
#[derive(Clone, Copy, Debug)]
pub struct LoadRequest<'a> {
    role: &'a str,
    tiers: &'a [Tier],
    approvals: &'a ApprovalSet,
    project_key: Option<&'a ProjectKey>,
}

impl<'a> LoadRequest<'a> {
    /// Load `role` from `tiers` (least to most specific), judging repository
    /// layers against `approvals`. Without [`Self::project_key`] no
    /// repository layer applies.
    pub fn new(role: &'a str, tiers: &'a [Tier], approvals: &'a ApprovalSet) -> Self {
        LoadRequest {
            role,
            tiers,
            approvals,
            project_key: None,
        }
    }

    /// The project the repository tiers belong to.
    pub fn project_key(mut self, key: &'a ProjectKey) -> Self {
        self.project_key = Some(key);
        self
    }
}

/// A layer that takes part in merging: already read, digested and parsed,
/// with its body and its bundle fragment files.
pub(crate) struct AppliedLayer {
    pub(crate) tier: String,
    pub(crate) tier_index: usize,
    pub(crate) digest: Digest,
    pub(crate) manifest_file: SourceRef,
    pub(crate) body: String,
    pub(crate) files: BTreeMap<String, String>,
    pub(crate) manifest: LayerManifest,
}

/// A body template and the lower body it extends, if any.
pub(crate) struct BodyTemplate {
    pub(crate) name: String,
    pub(crate) parent: Option<String>,
    pub(crate) origin: SourceRef,
    pub(crate) text: String,
    pub(crate) tier: String,
}

/// A bundle fragment chosen from the most specific tier that has it.
pub(crate) struct BundleFragment {
    pub(crate) tier: String,
    pub(crate) origin: SourceRef,
    pub(crate) text: String,
}

/// A role whose layers are read, approved and merged, ready to render.
pub struct LoadedRole {
    pub(crate) module: String,
    pub(crate) role: String,
    pub(crate) manifest: Manifest,
    pub(crate) tiers: Vec<TierReport>,
    pub(crate) parents: Vec<ParentReport>,
    /// The digests that enter the source revision, in ladder order:
    /// `(tier, role, digest)` for each applied layer, then each parent.
    pub(crate) layer_digests: Vec<(String, String, Digest)>,
    pub(crate) bodies: Vec<BodyTemplate>,
    pub(crate) entry: String,
    pub(crate) entry_tier: String,
    pub(crate) bundle_fragments: BTreeMap<String, BundleFragment>,
}

impl std::fmt::Debug for LoadedRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedRole")
            .field("module", &self.module)
            .field("role", &self.role)
            .field("manifest", &self.manifest)
            .field("tiers", &self.tiers)
            .field("entry_tier", &self.entry_tier)
            .finish_non_exhaustive()
    }
}

impl LoadedRole {
    /// The role id.
    pub fn role(&self) -> &str {
        &self.role
    }

    /// The composed manifest.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Each tier's entry in the resolution report, in ladder order.
    pub fn tiers(&self) -> &[TierReport] {
        &self.tiers
    }
}

impl PersonaEngine {
    /// An engine for `module`, a bare id that appears in approval keys.
    pub fn new(module: &str) -> Result<Self, PersonaError> {
        check_bare("module id", module)?;
        Ok(PersonaEngine {
            module: module.to_string(),
            registry: Registry::new(),
        })
    }

    /// The module id.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Register a top-level `PERSONA.md` key with its schema and merge rule.
    /// Keys are lowercase letters, digits and `_`, and cannot shadow a core
    /// field.
    pub fn register_extension<T>(&mut self, extension: Extension<T>) -> Result<(), PersonaError>
    where
        T: DeserializeOwned + Serialize + Clone + Send + Sync + 'static,
    {
        let key = extension.key;
        let well_formed = !key.is_empty()
            && key.len() <= 64
            && key.as_bytes()[0].is_ascii_lowercase()
            && key
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !well_formed {
            return Err(PersonaError::InvalidId {
                what: "extension key",
                value: key,
                reason: "must start with a lowercase letter and contain only lowercase letters, digits and `_`",
            });
        }
        if CORE_FIELDS.contains(&key.as_str()) {
            return Err(PersonaError::ExtensionConflict {
                key,
                reason: "it is a core field",
            });
        }
        if self.registry.contains_key(&key) {
            return Err(PersonaError::ExtensionConflict {
                key,
                reason: "it is already registered",
            });
        }
        self.registry.insert(
            key,
            Arc::new(Typed::<T> {
                required: extension.required,
                merge: extension.merge,
                _marker: PhantomData,
            }),
        );
        Ok(())
    }

    /// Read, approve and merge `request.role`'s layers.
    pub fn load(&self, request: LoadRequest<'_>) -> Result<LoadedRole, PersonaError> {
        let role = request.role;
        check_role(role)?;
        if request.tiers.is_empty() {
            return Err(PersonaError::NoTiers);
        }
        let mut names = BTreeSet::new();
        for tier in request.tiers {
            if !names.insert(tier.name()) {
                return Err(PersonaError::DuplicateTier {
                    tier: tier.name().to_string(),
                });
            }
        }

        let mut tier_reports = Vec::new();
        let mut applied = Vec::new();
        for (index, tier) in request.tiers.iter().enumerate() {
            let relative_path = tier.relative_path(role);
            let mut report = TierReport {
                name: tier.name().to_string(),
                trust: tier.trust(),
                root: tier.root().map(|p| p.to_string_lossy().into_owned()),
                relative_path: relative_path.clone(),
                present: false,
                approval: ApprovalState::NotRequired,
                layer_digest: None,
                used: false,
            };
            let Some(files) = tier.read_layer(role)? else {
                tier_reports.push(report);
                continue;
            };
            let digest = layer_digest(&files);
            report.present = true;
            report.layer_digest = Some(digest);
            report.approval = self.approval(request, tier, role, digest);
            if report.approval.applies() {
                // A layer that does not apply is skipped whole: it is not even
                // parsed, so nothing in it can affect the result.
                let manifest_file = tier.file_ref(role, MANIFEST);
                let parsed = parse_persona(&files[MANIFEST], &manifest_file, role, &self.registry)?;
                report.used = true;
                applied.push(AppliedLayer {
                    tier: tier.name().to_string(),
                    tier_index: index,
                    digest,
                    manifest_file,
                    body: parsed.body,
                    files,
                    manifest: parsed.manifest,
                });
            }
            tier_reports.push(report);
        }
        if applied.is_empty() {
            return Err(PersonaError::NoLayers {
                role: role.to_string(),
            });
        }

        let manifest = self.merge(role, &applied)?;
        let bundle_fragments = bundle_fragments(role, &manifest, request.tiers, &applied)?;
        let mut parents = Vec::new();
        let mut layer_digests: Vec<(String, String, Digest)> = applied
            .iter()
            .map(|l| (l.tier.clone(), role.to_string(), l.digest))
            .collect();
        let (bodies, entry) = self.bodies(request, role, &applied, &mut parents)?;
        for p in &parents {
            if let Some(digest) = p.layer_digest {
                layer_digests.push((p.tier.clone(), p.role.clone(), digest));
            }
        }
        // The entry template is the most specific body the role produced,
        // whatever lower body or role it extends.
        let entry = entry.ok_or_else(|| PersonaError::NoBody {
            role: role.to_string(),
        })?;
        let entry_tier = bodies
            .iter()
            .find(|b| b.name == entry)
            .map(|b| b.tier.clone())
            .expect("the entry is one of the bodies");
        Ok(LoadedRole {
            module: self.module.clone(),
            role: role.to_string(),
            manifest,
            tiers: tier_reports,
            parents,
            layer_digests,
            bodies,
            entry,
            entry_tier,
            bundle_fragments,
        })
    }

    fn approval(
        &self,
        request: LoadRequest<'_>,
        tier: &Tier,
        role: &str,
        digest: Digest,
    ) -> ApprovalState {
        match tier.trust() {
            crate::TrustClass::Operator => ApprovalState::NotRequired,
            crate::TrustClass::Repository => request.approvals.state(
                &self.module,
                role,
                request.project_key,
                &tier.relative_path(role),
                digest,
            ),
        }
    }

    /// Compose the applied layers: a present field replaces the inherited
    /// one, `disabled_blocks` unions, extensions use their registered rule.
    fn merge(&self, role: &str, applied: &[AppliedLayer]) -> Result<Manifest, PersonaError> {
        let first = &applied[0];
        for field in REQUIRED_FIELDS {
            if !first.manifest.present.contains(field) {
                return Err(PersonaError::MissingField {
                    file: first.manifest_file.clone(),
                    field: field.to_string(),
                });
            }
        }
        for (key, ext) in &self.registry {
            if ext.required() && !first.manifest.extensions.contains_key(key) {
                return Err(PersonaError::MissingField {
                    file: first.manifest_file.clone(),
                    field: key.clone(),
                });
            }
        }

        let mut version = 0;
        let mut display_name = String::new();
        let mut fragments = Vec::new();
        let mut disabled_blocks = BTreeSet::new();
        let mut extensions: BTreeMap<String, AnyBox> = BTreeMap::new();
        for layer in applied {
            let m = &layer.manifest;
            if let Some(v) = m.version {
                version = v;
            }
            if let Some(d) = &m.display_name {
                display_name = d.clone();
            }
            if let Some(f) = &m.fragments {
                fragments = f.clone();
            }
            if let Some(blocks) = &m.disabled_blocks {
                disabled_blocks.extend(blocks.iter().cloned());
            }
            for (key, ext) in &self.registry {
                let inherited = extensions.remove(key);
                let merged = ext.merge(key, inherited, &m.extensions).map_err(|reason| {
                    PersonaError::ExtensionMerge {
                        file: layer.manifest_file.clone(),
                        field: key.clone(),
                        reason,
                    }
                })?;
                if let Some(value) = merged {
                    extensions.insert(key.clone(), value);
                }
            }
        }

        let last = &applied[applied.len() - 1];
        let mut composed = BTreeMap::new();
        for (key, value) in extensions {
            let ext = self.registry[&key].clone();
            // Checked here so the composed manifest always serialises.
            ext.to_json(&value)
                .map_err(|reason| PersonaError::ExtensionMerge {
                    file: last.manifest_file.clone(),
                    field: key.clone(),
                    reason,
                })?;
            composed.insert(key, (ext, value));
        }
        Ok(Manifest {
            name: role.to_string(),
            version,
            display_name,
            fragments,
            disabled_blocks,
            extensions: composed,
        })
    }

    /// Name every non-empty body and link it to the body it extends: its
    /// declared `extends` parent, or else the previous non-empty body.
    fn bodies(
        &self,
        request: LoadRequest<'_>,
        role: &str,
        applied: &[AppliedLayer],
        parents: &mut Vec<ParentReport>,
    ) -> Result<(Vec<BodyTemplate>, Option<String>), PersonaError> {
        let mut bodies: Vec<BodyTemplate> = Vec::new();
        let mut previous: Option<String> = None;
        for layer in applied {
            let has_body = !layer.body.trim().is_empty();
            let Some(extends) = &layer.manifest.extends else {
                if has_body {
                    let name = format!("@body/{}", layer.tier);
                    bodies.push(BodyTemplate {
                        name: name.clone(),
                        parent: previous.take(),
                        origin: layer.manifest_file.clone(),
                        text: layer.body.clone(),
                        tier: layer.tier.clone(),
                    });
                    previous = Some(name);
                }
                continue;
            };
            let unavailable = |reason: &'static str| PersonaError::ExtendsUnavailable {
                file: layer.manifest_file.clone(),
                role: extends.role.clone(),
                tier: extends.tier.clone(),
                reason,
            };
            if !has_body {
                return Err(PersonaError::InvalidField {
                    file: layer.manifest_file.clone(),
                    field: "extends".into(),
                    reason: "names the parent of this layer's body, but the body is empty".into(),
                });
            }
            let parent_index = request
                .tiers
                .iter()
                .position(|t| t.name() == extends.tier)
                .ok_or_else(|| unavailable("no tier has that name"))?;
            if parent_index >= layer.tier_index {
                return Err(unavailable("the parent must be in a lower tier"));
            }
            let parent_name = if extends.role == role {
                let parent = applied
                    .iter()
                    .find(|l| l.tier_index == parent_index)
                    .ok_or_else(|| unavailable("that tier has no applied layer for the role"))?;
                if parent.body.trim().is_empty() {
                    return Err(unavailable("that layer's body is empty"));
                }
                format!("@body/{}", parent.tier)
            } else {
                let name = format!("@body/{}/{}", extends.tier, extends.role);
                if !bodies.iter().any(|b| b.name == name) {
                    let body = self.foreign_parent(
                        request,
                        &request.tiers[parent_index],
                        &extends.role,
                        parents,
                        &unavailable,
                    )?;
                    bodies.push(BodyTemplate {
                        name: name.clone(),
                        parent: None,
                        origin: request.tiers[parent_index].file_ref(&extends.role, MANIFEST),
                        text: body,
                        tier: extends.tier.clone(),
                    });
                }
                name
            };
            let name = format!("@body/{}", layer.tier);
            bodies.push(BodyTemplate {
                name: name.clone(),
                parent: Some(parent_name),
                origin: layer.manifest_file.clone(),
                text: layer.body.clone(),
                tier: layer.tier.clone(),
            });
            previous = Some(name);
        }
        Ok((bodies, previous))
    }

    /// Load another role's layer to serve as an `extends` parent. It is held
    /// to the same approval rule as any layer, contributes only its body, and
    /// may not itself declare `extends`.
    fn foreign_parent(
        &self,
        request: LoadRequest<'_>,
        tier: &Tier,
        role: &str,
        parents: &mut Vec<ParentReport>,
        unavailable: &dyn Fn(&'static str) -> PersonaError,
    ) -> Result<String, PersonaError> {
        let Some(files) = tier.read_layer(role)? else {
            return Err(unavailable("that tier has no PERSONA.md for the role"));
        };
        let digest = layer_digest(&files);
        let approval = self.approval(request, tier, role, digest);
        parents.push(ParentReport {
            role: role.to_string(),
            tier: tier.name().to_string(),
            relative_path: tier.relative_path(role),
            approval,
            layer_digest: Some(digest),
        });
        if !approval.applies() {
            return Err(unavailable("that layer is not approved for this project"));
        }
        let file = tier.file_ref(role, MANIFEST);
        let parsed = parse_persona(&files[MANIFEST], &file, role, &self.registry)?;
        if parsed.manifest.extends.is_some() {
            return Err(unavailable("that layer declares its own `extends`"));
        }
        if parsed.body.trim().is_empty() {
            return Err(unavailable("that layer's body is empty"));
        }
        Ok(parsed.body)
    }
}

/// For each declared bare fragment, the file from the most specific applied
/// layer that has it. Qualified ids are never looked up in a bundle.
fn bundle_fragments(
    role: &str,
    manifest: &Manifest,
    tiers: &[Tier],
    applied: &[AppliedLayer],
) -> Result<BTreeMap<String, BundleFragment>, PersonaError> {
    let mut out = BTreeMap::new();
    for id in manifest.fragments.iter().filter(|f| !f.is_qualified()) {
        let logical = format!("fragments/{id}.md");
        debug_assert!(fragment_file_id(&logical).is_some());
        let found = applied
            .iter()
            .rev()
            .find_map(|l| l.files.get(&logical).map(|text| (l, text)));
        let Some((layer, text)) = found else {
            return Err(PersonaError::MissingFragment {
                role: role.to_string(),
                fragment: id.to_string(),
            });
        };
        out.insert(
            id.to_string(),
            BundleFragment {
                tier: layer.tier.clone(),
                origin: tiers[layer.tier_index].file_ref(role, &logical),
                text: text.clone(),
            },
        );
    }
    Ok(out)
}
