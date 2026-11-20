//! Provider-authored views of notice record data (contract §2.1).

use cortexkit_role_tool_display::{
    notice_view_key, parse_notice_view_key, valid_notice_view_name, valid_notice_view_version,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use subc_protocol::ErrorBody;

use crate::{catalog::check_flat_schema, errors};

/// The notice definition pinned by its writer, name and version.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NoticeViewDefinition {
    pub name: String,
    pub version: u64,
    pub data_schema: Value,
}

impl NoticeViewDefinition {
    pub fn new(name: impl Into<String>, version: u64, data_schema: Value) -> Self {
        Self {
            name: name.into(),
            version,
            data_schema,
        }
    }
}

/// Opaque template JSON for the existing `tool_row` and `tool_page` profiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NoticeViewTemplates {
    pub row: Value,
    pub page: Value,
}

impl NoticeViewTemplates {
    pub fn new(row: Value, page: Value) -> Self {
        Self { row, page }
    }
}

/// The entry served by `tool.views`. As for tool views, the writer's views
/// bucket stores the one-pin `tool.views` reply, `{"views": {"nv1:…": <entry>}}`,
/// under the lowercase hex of the pin's UTF-8 bytes, not the bare entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NoticeView {
    pub notice: NoticeViewDefinition,
    pub template: NoticeViewTemplates,
    pub template_revision: u64,
}

impl NoticeView {
    pub fn new(
        notice: NoticeViewDefinition,
        template: NoticeViewTemplates,
        template_revision: u64,
    ) -> Self {
        Self {
            notice,
            template,
            template_revision,
        }
    }

    /// Check the entry against the qualified pin and the attested publisher.
    /// This checks schema flatness, not the template language or its schema subset.
    pub fn check_pin(&self, writer: &str, pin: &str) -> Result<(), ErrorBody> {
        let owner = self.check_entry(pin)?;
        if owner != writer {
            return Err(errors::invalid_request(
                &format!("views.{pin}"),
                "notice pin names a different writer",
            ));
        }
        Ok(())
    }

    /// Lowercase hex of the pin's UTF-8 bytes, for the writer's views bucket.
    pub fn bucket_key(&self, writer: &str) -> Result<String, ErrorBody> {
        let pin = notice_view_key(writer, &self.notice.name, self.notice.version)
            .ok_or_else(|| errors::invalid_request("notice", "invalid notice pin"))?;
        Ok(pin.bytes().map(|byte| format!("{byte:02x}")).collect())
    }

    pub(super) fn check_entry<'a>(&self, pin: &'a str) -> Result<&'a str, ErrorBody> {
        let invalid = |field: &str, reason: &str| {
            errors::invalid_request(&format!("views.{pin}{field}"), reason)
        };
        let (writer, name, version) = parse_notice_view_key(pin)
            .ok_or_else(|| invalid("", "invalid qualified notice pin"))?;
        if !valid_notice_view_name(&self.notice.name) || self.notice.name != name {
            return Err(invalid(
                ".notice.name",
                "name does not match the notice pin",
            ));
        }
        if !valid_notice_view_version(self.notice.version) || self.notice.version != version {
            return Err(invalid(
                ".notice.version",
                "version does not match the notice pin",
            ));
        }
        check_flat_schema(&self.notice.data_schema)
            .map_err(|_| invalid(".notice.data_schema", "expected a flat schema"))?;
        Ok(writer)
    }
}
