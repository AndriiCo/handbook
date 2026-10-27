//! Content-repo template registry (P2-F4, P2-F5).
//!
//! Files live at `forms/<authority>/<code>/v<n>.json`; each file holds
//! one [`FormTemplate`](crate::FormTemplate). The registry loads the
//! tree, serves the active edition per (`authority`, `code`), and
//! publishes new editions with supersede chaining plus a changed-key
//! diff for in-flight users (P2-U2).

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;

use crate::template::{FormTemplate, TemplateError, TemplateStatus};

/// Changed-key diff between two editions (P2-F4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TemplateDiff {
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub changed: Vec<String>,
}

impl TemplateDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

/// Field-level diff: keys only in `next` are added, only in `current`
/// are removed, and keys whose type/required/options/pattern changed
/// are changed (label/help-only edits auto-carry, P2-F4).
pub fn diff_fields(current: &FormTemplate, next: &FormTemplate) -> TemplateDiff {
    let old: BTreeMap<&str, _> = current.fields.iter().map(|f| (f.key.as_str(), f)).collect();
    let new: BTreeMap<&str, _> = next.fields.iter().map(|f| (f.key.as_str(), f)).collect();
    let mut diff = TemplateDiff::default();
    for key in new.keys().filter(|k| !old.contains_key(*k)) {
        diff.added.push((*key).to_string());
    }
    for key in old.keys().filter(|k| !new.contains_key(*k)) {
        diff.removed.push((*key).to_string());
    }
    for key in new.keys().filter(|k| old.contains_key(*k)) {
        let a = old[key];
        let b = new[key];
        if a.field_type != b.field_type
            || a.required != b.required
            || a.options != b.options
            || a.pattern != b.pattern
            || a.max_len != b.max_len
        {
            diff.changed.push((*key).to_string());
        }
    }
    diff.added.sort();
    diff.removed.sort();
    diff.changed.sort();
    diff
}

/// In-memory index over the content-repo template files.
#[derive(Debug, Default)]
pub struct TemplateRegistry {
    templates: Vec<FormTemplate>,
}

impl TemplateRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load every `forms/<authority>/<code>/v<n>.json` under `root`.
    ///
    /// Files that fail to parse are returned as the error (fail-closed:
    /// a half-loaded registry must never serve).
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] on I/O failure, invalid JSON, or a
    /// template whose schema is invalid.
    pub fn load_dir(root: &Path) -> Result<Self, RegistryError> {
        let mut registry = Self::new();
        let mut files = Vec::new();
        collect_json(root, &mut files).map_err(RegistryError::Io)?;
        files.sort();
        for path in files {
            let bytes = std::fs::read(&path).map_err(RegistryError::Io)?;
            let template: FormTemplate =
                serde_json::from_slice(&bytes).map_err(|e| RegistryError::Parse {
                    path: path.clone(),
                    message: e.to_string(),
                })?;
            template
                .validate_schema()
                .map_err(|e| RegistryError::Parse {
                    path: path.clone(),
                    message: e.to_string(),
                })?;
            registry.templates.push(template);
        }
        registry.check_single_active()?;
        Ok(registry)
    }

    /// The active edition for one authority form, if any.
    pub fn active(&self, authority: &str, code: &str) -> Option<&FormTemplate> {
        self.templates.iter().find(|t| {
            t.authority == authority && t.code == code && t.status == TemplateStatus::Active
        })
    }

    /// All editions of one authority form, oldest first.
    pub fn editions(&self, authority: &str, code: &str) -> Vec<&FormTemplate> {
        let mut out: Vec<&FormTemplate> = self
            .templates
            .iter()
            .filter(|t| t.authority == authority && t.code == code)
            .collect();
        out.sort_by_key(|t| t.version);
        out
    }

    /// Publish a new edition: the current active (if any) flips to
    /// `superseded`, the new edition becomes `active` with
    /// `version = old + 1`, and the changed-key diff is returned for
    /// the in-flight-user notice (P2-U2).
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] when the schema is invalid or the
    /// version does not continue the chain.
    pub fn publish(&mut self, mut next: FormTemplate) -> Result<TemplateDiff, RegistryError> {
        next.validate_schema().map_err(RegistryError::Invalid)?;
        let current = self
            .editions(&next.authority.clone(), &next.code.clone())
            .into_iter()
            .last()
            .cloned();
        let diff = match current {
            None => {
                if next.version != 1 {
                    return Err(RegistryError::VersionGap { expected: 1 });
                }
                diff_fields(
                    &FormTemplate {
                        fields: Vec::new(),
                        ..next.clone()
                    },
                    &next,
                )
            }
            Some(current) => {
                if next.version != current.version + 1 {
                    return Err(RegistryError::VersionGap {
                        expected: current.version + 1,
                    });
                }
                let diff = diff_fields(&current, &next);
                for t in &mut self.templates {
                    if t.authority == next.authority
                        && t.code == next.code
                        && t.status == TemplateStatus::Active
                    {
                        t.status = TemplateStatus::Superseded;
                    }
                }
                diff
            }
        };
        next.status = TemplateStatus::Active;
        self.templates.push(next);
        Ok(diff)
    }

    fn check_single_active(&self) -> Result<(), RegistryError> {
        let mut active: BTreeMap<(&str, &str), u32> = BTreeMap::new();
        for t in &self.templates {
            if t.status == TemplateStatus::Active
                && active
                    .insert((t.authority.as_str(), t.code.as_str()), t.version)
                    .is_some()
            {
                return Err(RegistryError::DuplicateActive {
                    authority: t.authority.clone(),
                    code: t.code.clone(),
                });
            }
        }
        Ok(())
    }
}

fn collect_json(dir: &Path, out: &mut Vec<std::path::PathBuf>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_json(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "json") {
            out.push(path);
        }
    }
    Ok(())
}

/// [`TemplateRegistry`] failure.
#[derive(Debug)]
#[non_exhaustive]
pub enum RegistryError {
    Io(std::io::Error),
    Parse {
        path: std::path::PathBuf,
        message: String,
    },
    Invalid(TemplateError),
    DuplicateActive {
        authority: String,
        code: String,
    },
    VersionGap {
        expected: u32,
    },
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RegistryError::Io(e) => write!(f, "template registry io: {e}"),
            RegistryError::Parse { path, message } => {
                write!(f, "template {}: {message}", path.display())
            }
            RegistryError::Invalid(e) => write!(f, "invalid template: {e}"),
            RegistryError::DuplicateActive { authority, code } => {
                write!(f, "two active editions of {authority}/{code}")
            }
            RegistryError::VersionGap { expected } => {
                write!(f, "next edition must be version {expected}")
            }
        }
    }
}

impl std::error::Error for RegistryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::FieldType;

    fn sample() -> FormTemplate {
        crate::template::tests::sample_template()
    }

    #[test]
    fn json_round_trip() {
        let template = sample();
        let json = serde_json::to_string_pretty(&template).unwrap();
        let back: FormTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back, template);
    }

    #[test]
    fn load_dir_round_trip() {
        let dir = tempfile_dir();
        let template = sample();
        let json = serde_json::to_string(&template).unwrap();
        let path = dir.join("forms/usa-wa-sos/wa-apostille-request/v1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, json).unwrap();
        let registry = TemplateRegistry::load_dir(&dir.join("forms")).unwrap();
        let editions = registry.editions("usa-wa-sos", "wa-apostille-request");
        assert_eq!(editions.len(), 1);
        assert_eq!(editions[0].version, 1);
    }

    #[test]
    fn publish_chains_and_diffs() {
        let mut registry = TemplateRegistry::new();
        let mut v1 = sample();
        v1.version = 1;
        let diff = registry.publish(v1).unwrap();
        assert_eq!(diff.added, vec!["full_name".to_string()]);

        let mut v2 = sample();
        v2.version = 2;
        v2.fields[0].field_type = FieldType::Textarea;
        v2.fields.push(new_field("email", FieldType::Email));
        let diff = registry.publish(v2).unwrap();
        assert_eq!(diff.added, vec!["email".to_string()]);
        assert!(diff.removed.is_empty());
        assert_eq!(diff.changed, vec!["full_name".to_string()]);

        let active = registry
            .active("usa-wa-sos", "wa-apostille-request")
            .unwrap();
        assert_eq!(active.version, 2);
        let editions = registry.editions("usa-wa-sos", "wa-apostille-request");
        assert_eq!(editions[0].status, TemplateStatus::Superseded);
    }

    #[test]
    fn publish_rejects_version_gap() {
        let mut registry = TemplateRegistry::new();
        let mut v3 = sample();
        v3.version = 3;
        assert!(matches!(
            registry.publish(v3).unwrap_err(),
            RegistryError::VersionGap { expected: 1 }
        ));
    }

    fn new_field(key: &str, field_type: FieldType) -> crate::template::FieldDef {
        let mut field = sample().fields[0].clone();
        field.key = key.to_string();
        field.label = key.to_string();
        field.field_type = field_type;
        field.required = false;
        field
    }

    fn tempfile_dir() -> std::path::PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!("forms-registry-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
