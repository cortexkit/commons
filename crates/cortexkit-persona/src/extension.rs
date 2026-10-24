//! Module-registered top-level keys: a typed schema and a merge rule each.

use std::{any::Any, collections::BTreeMap, marker::PhantomData, sync::Arc};

use serde::{de::DeserializeOwned, Serialize};

pub(crate) type AnyBox = Box<dyn Any + Send + Sync>;

/// A closure merge rule: see [`MergeRule::Custom`].
pub type MergeFn<T> = Arc<dyn Fn(MergeInput<'_, T>) -> Result<Option<T>, String> + Send + Sync>;

/// How an extension's value composes across layers.
pub enum MergeRule<T> {
    /// A present value replaces the inherited one, like a core field.
    Replace,
    /// Called once for every applied layer, least to most specific, whether
    /// or not the layer has this key, so a rule can also read a sibling key of
    /// the same layer (a removal list, for example). Returns the composed
    /// value after this layer; `Err` refuses the layer with that message.
    Custom(MergeFn<T>),
}

impl<T> Clone for MergeRule<T> {
    fn clone(&self) -> Self {
        match self {
            MergeRule::Replace => MergeRule::Replace,
            MergeRule::Custom(f) => MergeRule::Custom(f.clone()),
        }
    }
}

/// What a [`MergeRule::Custom`] rule sees for one layer.
pub struct MergeInput<'a, T> {
    /// The value composed from the lower layers.
    pub inherited: Option<T>,
    /// This layer's value, if the layer has the key.
    pub present: Option<T>,
    /// Every extension value of this layer, for reading sibling keys.
    pub layer: LayerExtensions<'a>,
}

/// The extension values present in one layer, typed by their registration.
#[derive(Clone, Copy)]
pub struct LayerExtensions<'a> {
    values: &'a BTreeMap<String, AnyBox>,
}

impl<'a> LayerExtensions<'a> {
    pub(crate) fn new(values: &'a BTreeMap<String, AnyBox>) -> Self {
        LayerExtensions { values }
    }

    /// The layer's value for `key`, if present and registered with type `U`.
    pub fn get<U: 'static>(&self, key: &str) -> Option<&'a U> {
        self.values.get(key).and_then(|v| v.downcast_ref::<U>())
    }
}

/// A top-level `PERSONA.md` key a module registers with
/// [`crate::PersonaEngine::register_extension`].
///
/// `T` is the key's schema: its `Deserialize` impl is the validation, so use
/// `#[serde(deny_unknown_fields)]` to keep nested objects closed. Its
/// `Serialize` form enters the composed manifest and so the source revision.
pub struct Extension<T> {
    pub(crate) key: String,
    pub(crate) required: bool,
    pub(crate) merge: MergeRule<T>,
}

impl<T> Extension<T>
where
    T: DeserializeOwned + Serialize + Clone + Send + Sync + 'static,
{
    /// An optional key that replaces like a core field.
    pub fn new(key: &str) -> Self {
        Extension {
            key: key.to_string(),
            required: false,
            merge: MergeRule::Replace,
        }
    }

    /// The first applied layer must supply the key.
    pub fn required(mut self) -> Self {
        self.required = true;
        self
    }

    /// Use `rule` to compose the key across layers.
    pub fn merge(mut self, rule: MergeRule<T>) -> Self {
        self.merge = rule;
        self
    }
}

/// The type-erased form the engine stores.
pub(crate) trait ErasedExtension: Send + Sync {
    fn required(&self) -> bool;
    fn parse(&self, value: yaml_serde::Value) -> Result<AnyBox, String>;
    /// Compose one layer. `layer_values` holds every extension value the
    /// layer has; a custom rule reads them through [`LayerExtensions`].
    fn merge(
        &self,
        key: &str,
        inherited: Option<AnyBox>,
        layer_values: &BTreeMap<String, AnyBox>,
    ) -> Result<Option<AnyBox>, String>;
    fn to_json(&self, value: &AnyBox) -> Result<serde_json::Value, String>;
}

pub(crate) struct Typed<T> {
    pub(crate) required: bool,
    pub(crate) merge: MergeRule<T>,
    pub(crate) _marker: PhantomData<fn() -> T>,
}

impl<T> ErasedExtension for Typed<T>
where
    T: DeserializeOwned + Serialize + Clone + Send + Sync + 'static,
{
    fn required(&self) -> bool {
        self.required
    }

    fn parse(&self, value: yaml_serde::Value) -> Result<AnyBox, String> {
        let typed: T = yaml_serde::from_value(value).map_err(|e| e.to_string())?;
        Ok(Box::new(typed))
    }

    fn merge(
        &self,
        key: &str,
        inherited: Option<AnyBox>,
        layer_values: &BTreeMap<String, AnyBox>,
    ) -> Result<Option<AnyBox>, String> {
        let inherited = inherited.map(|b| *b.downcast::<T>().expect("registered type"));
        let present = layer_values
            .get(key)
            .map(|b| b.downcast_ref::<T>().expect("registered type").clone());
        let out = match &self.merge {
            MergeRule::Replace => present.or(inherited),
            MergeRule::Custom(rule) => rule(MergeInput {
                inherited,
                present,
                layer: LayerExtensions::new(layer_values),
            })?,
        };
        Ok(out.map(|v| Box::new(v) as AnyBox))
    }

    fn to_json(&self, value: &AnyBox) -> Result<serde_json::Value, String> {
        let typed = value.downcast_ref::<T>().expect("registered type");
        serde_json::to_value(typed).map_err(|e| e.to_string())
    }
}
