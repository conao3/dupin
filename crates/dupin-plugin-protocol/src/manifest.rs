use crate::{Operation, SystemCommand, View};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub const SCHEMA_VERSION: u32 = 1;
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Capability {
    ClipboardWrite,
    SystemSleep,
    SystemLockScreen,
    CalendarRead,
    OpenUrl(crate::url_scope::UrlScope),
}
impl Capability {
    pub fn id(&self) -> String {
        match self {
            Self::OpenUrl(scope) => return scope.id(),
            Self::ClipboardWrite => "clipboard-write",
            Self::SystemSleep => "system-sleep",
            Self::SystemLockScreen => "system-lock-screen",
            Self::CalendarRead => "calendar-read",
        }
        .into()
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    CopyText,
    OpenUrl,
    Sleep,
    LockScreen,
}
impl ActionKind {
    pub fn capability(self) -> Option<Capability> {
        Some(match self {
            Self::OpenUrl => return None,
            Self::CopyText => Capability::ClipboardWrite,
            Self::Sleep => Capability::SystemSleep,
            Self::LockScreen => Capability::SystemLockScreen,
        })
    }
    pub fn of(op: &Operation) -> Self {
        match op {
            Operation::OpenUrl { .. } => Self::OpenUrl,
            Operation::CopyText { .. } => Self::CopyText,
            Operation::SystemCommand {
                command: SystemCommand::Sleep,
            } => Self::Sleep,
            Operation::SystemCommand {
                command: SystemCommand::LockScreen,
            } => Self::LockScreen,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewKind {
    Text,
    Detail,
    Pair,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Eligibility {
    Nonempty,
}
impl Eligibility {
    pub fn accepts(self, query: &str) -> bool {
        !query.trim().is_empty()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDescriptor {
    pub id: String,
    pub title: String,
    pub description: String,
    pub kind: Capability,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub id: String,
    pub title: String,
    pub description: String,
    pub plugin_version: String,
    pub command: String,
    pub entrypoint: String,
    pub eligibility: Eligibility,
    pub result_limit: u32,
    pub default_enabled: bool,
    pub capabilities: Vec<CapabilityDescriptor>,
    pub views: Vec<ViewKind>,
    pub actions: Vec<ActionKind>,
}
#[derive(Clone, Debug)]
pub struct ValidatedManifest(Manifest);
impl std::ops::Deref for ValidatedManifest {
    type Target = Manifest;
    fn deref(&self) -> &Manifest {
        &self.0
    }
}
fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}
fn text(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && !s.chars().any(char::is_control)
}
fn unique<T: Ord + Copy>(v: &[T]) -> bool {
    v.iter().copied().collect::<BTreeSet<_>>().len() == v.len()
}
impl Manifest {
    pub fn decode(bytes: &[u8]) -> Result<ValidatedManifest, &'static str> {
        if bytes.len() > 16384 {
            return Err("manifest too large");
        }
        let manifest: Self = serde_json::from_slice(bytes).map_err(|_| "invalid manifest")?;
        manifest.validate()
    }
    pub fn validate(self) -> Result<ValidatedManifest, &'static str> {
        if self.schema_version != SCHEMA_VERSION {
            return Err("unsupported manifest version");
        }
        let parts: Vec<_> = self.command.split('.').collect();
        if !identifier(&self.id)
            || parts.len() != 2
            || !parts.iter().all(|p| identifier(p))
            || !identifier(&self.entrypoint)
            || !text(&self.plugin_version, 64)
            || !text(&self.title, 128)
            || !text(&self.description, 1024)
            || self.result_limit == 0
            || self.result_limit > crate::MAX_RESULTS
        {
            return Err("invalid manifest metadata");
        }
        if self.capabilities.len() > 16
            || self.views.is_empty()
            || self.views.len() > 3
            || self.actions.len() > 8
            || !unique(&self.views)
            || !unique(&self.actions)
        {
            return Err("invalid declarations");
        }
        let mut caps = BTreeSet::new();
        for cap in &self.capabilities {
            if cap.id != cap.kind.id()
                || !caps.insert(cap.kind.clone())
                || !text(&cap.title, 128)
                || !text(&cap.description, 1024)
            {
                return Err("invalid capability identity");
            }
        }
        if self.actions.iter().any(|a| match a.capability() {
            Some(cap) => !caps.contains(&cap),
            None => !caps.iter().any(|c| matches!(c, Capability::OpenUrl(_))),
        }) {
            return Err("undeclared action capability");
        }
        Ok(ValidatedManifest(self))
    }
}
pub fn validate_catalog(manifests: &[ValidatedManifest], reserved: &[&str]) -> Result<(), &'static str> {
    if manifests.len() > 64 {
        return Err("too many plugins");
    }
    let mut ids = BTreeSet::new();
    let mut commands: BTreeSet<&str> = reserved.iter().copied().collect();
    for m in manifests {
        if !ids.insert(m.id.as_str()) {
            return Err("duplicate plugin identity");
        }
        if !commands.insert(m.command.as_str()) {
            return Err("command collision");
        }
    }
    Ok(())
}
impl ValidatedManifest {
    pub fn allows_operation(&self, op: &Operation) -> bool {
        self.actions.contains(&ActionKind::of(op))
            && match op {
                Operation::OpenUrl { url } => self
                    .capabilities
                    .iter()
                    .any(|c| matches!(&c.kind, Capability::OpenUrl(scope) if scope.allows(url))),
                _ => true,
            }
    }
    pub fn allows_view(&self, view: &View) -> bool {
        self.views.contains(&match view {
            View::Text { .. } => ViewKind::Text,
            View::Detail { .. } => ViewKind::Detail,
            View::Pair { .. } => ViewKind::Pair,
        })
    }
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(&self.0).expect("valid manifest encoding")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn value() -> serde_json::Value {
        serde_json::from_slice(include_bytes!("../../dupin/src/calculator-manifest.json")).unwrap()
    }
    fn decode(v: &serde_json::Value) -> Result<ValidatedManifest, &'static str> {
        Manifest::decode(&serde_json::to_vec(v).unwrap())
    }
    #[test]
    fn url_actions_require_matching_validated_scopes_and_ids() {
        let mut v = value();
        v["actions"] = serde_json::json!(["open_url"]);
        assert!(decode(&v).is_err());
        v["capabilities"] = serde_json::json!([{
            "id": "open-url:https:example.com", "title": "Open example",
            "description": "Open example URLs", "kind": {"open-url": {"scheme": "HTTPS", "host": "EXAMPLE.COM"}}
        }]);
        let manifest = decode(&v).unwrap();
        assert!(manifest.allows_operation(&Operation::OpenUrl {
            url: "https://example.com/path".into()
        }));
        assert!(!manifest.allows_operation(&Operation::OpenUrl {
            url: "https://example.com.evil/".into()
        }));
        for (scheme, host) in [
            ("https", serde_json::Value::Null),
            ("file", serde_json::Value::Null),
            ("https", "example.com:443".into()),
            ("https", "%65xample.com".into()),
        ] {
            let mut bad = v.clone();
            bad["capabilities"][0]["kind"]["open-url"]["scheme"] = scheme.into();
            bad["capabilities"][0]["kind"]["open-url"]["host"] = host;
            assert!(decode(&bad).is_err());
        }
        v["capabilities"][0]["kind"]["open-url"]["host"] = "other.example".into();
        assert!(decode(&v).is_err());
        let calculator = decode(&value()).unwrap();
        assert!(!calculator.allows_operation(&Operation::OpenUrl {
            url: "dict://term".into()
        }));
    }
    #[test]
    fn current_manifests_roundtrip() {
        for bytes in [
            include_bytes!("../../dupin/src/calculator-manifest.json").as_slice(),
            include_bytes!("../../dupin/src/system-manifest.json").as_slice(),
        ] {
            let m = Manifest::decode(bytes).unwrap();
            assert_eq!(m.encode(), Manifest::decode(&m.encode()).unwrap().encode());
        }
    }
    #[test]
    fn version_and_unknown_fields_fail_closed() {
        for pointer in ["", "/capabilities/0"] {
            let mut v = value();
            v.pointer_mut(pointer)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("future".into(), true.into());
            assert!(decode(&v).is_err());
        }
        let mut v = value();
        v["schema_version"] = 99.into();
        assert!(decode(&v).is_err());
    }
    #[test]
    fn identity_and_command_collisions() {
        let m = decode(&value()).unwrap();
        assert!(validate_catalog(&[m.clone(), m.clone()], &[]).is_err());
        assert!(validate_catalog(std::slice::from_ref(&m), &["calculator.evaluate"]).is_err());
        let mut v = value();
        v["id"] = "other".into();
        assert!(validate_catalog(&[m, decode(&v).unwrap()], &[]).is_err());
    }
    #[test]
    fn bounds_identifiers_and_declarations() {
        for (field, val) in [
            ("id", "Bad ID".into()),
            ("command", "a.b.c".into()),
            ("title", "x".repeat(129).into()),
            ("result_limit", 501.into()),
        ] {
            let mut v = value();
            v[field] = val;
            assert!(decode(&v).is_err());
        }
        let mut v = value();
        let cap = v["capabilities"][0].clone();
        v["capabilities"].as_array_mut().unwrap().push(cap);
        assert!(decode(&v).is_err());
        let mut v = value();
        v["capabilities"] = serde_json::json!([]);
        assert!(decode(&v).is_err());
    }
    #[test]
    fn approved_id_cannot_be_reassigned_to_another_privilege() {
        let mut v = value();
        v["capabilities"][0]["kind"] = "system-sleep".into();
        v["actions"] = serde_json::json!(["sleep"]);
        assert!(decode(&v).is_err());
    }
    #[test]
    fn undeclared_guest_views_and_operations_are_rejected() {
        let m = decode(&value()).unwrap();
        assert!(!m.allows_view(&View::Detail {
            title: "a".into(),
            body: "b".into()
        }));
        assert!(!m.allows_operation(&Operation::SystemCommand {
            command: SystemCommand::Sleep
        }));
        assert!(m.allows_operation(&Operation::CopyText {
            text: "result".into()
        }));
    }
}
