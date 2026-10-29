//! General instruction template pack (TOML, versioned in repo).
//!
//! The template states *general* requirements; per-authority
//! findings supply the particular. Section→topic mapping is fixed
//! here — a section whose topic is not in the frozen enum fails
//! the pack at load, never at review time.

use std::collections::HashSet;
use std::path::Path;

use serde::Deserialize;

use crate::error::{Error, Result};

/// Template pack file.
#[derive(Debug, Clone, Deserialize)]
pub struct TemplatePack {
    pub version: String,
    #[serde(default)]
    pub sections: Vec<TemplateSection>,
    #[serde(default)]
    pub worker: WorkerConfig,
}

/// One instruction section.
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateSection {
    /// Stable id (`fees`, `walkin`, …).
    pub id: String,
    pub title: String,
    /// Frozen fact topic this section proposes under.
    pub topic: String,
    /// General requirement guidance for the drafter.
    pub guidance: String,
}

/// Worker knobs (model + budgets).
#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    /// Atlas model table (window, RPM, prices).
    #[serde(default)]
    pub models: Vec<atlas::config::ModelSpec>,
    /// Default model alias.
    #[serde(default = "default_model")]
    pub model: String,
    /// Evidence over this many estimator-tokens gets summarized.
    #[serde(default = "default_summarize_at")]
    pub summarize_at_tokens: u32,
    /// Max triggers per authority per sweep.
    #[serde(default = "default_max_triggers")]
    pub max_triggers: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            models: Vec::new(),
            model: default_model(),
            summarize_at_tokens: default_summarize_at(),
            max_triggers: default_max_triggers(),
        }
    }
}

fn default_model() -> String {
    "spark".to_string()
}

fn default_summarize_at() -> u32 {
    12_000
}

fn default_max_triggers() -> usize {
    10
}

impl TemplatePack {
    /// Load + validate a template pack.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(Error::Io)?;
        let pack: TemplatePack =
            toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        pack.validate()?;
        Ok(pack)
    }

    fn validate(&self) -> Result<()> {
        if self.version.trim().is_empty() {
            return Err(Error::Config("template version is empty".to_string()));
        }
        let allowed: HashSet<&str> = contracts::topics::Topic::ALL
            .iter()
            .map(|t| t.as_str())
            .collect();
        let mut ids = HashSet::new();
        for section in &self.sections {
            if !ids.insert(section.id.as_str()) {
                return Err(Error::Config(format!("duplicate section '{}'", section.id)));
            }
            if !allowed.contains(section.topic.as_str()) {
                return Err(Error::Config(format!(
                    "section '{}': topic '{}' is not a frozen fact topic",
                    section.id, section.topic
                )));
            }
        }
        Ok(())
    }

    /// Render the template for a draft prompt (pack order).
    pub fn render(&self) -> String {
        self.sections
            .iter()
            .map(|s| {
                format!(
                    "[{}] {} (topic: {})\n{}",
                    s.id, s.title, s.topic, s.guidance
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Look up the model spec by alias.
    pub fn model(&self, name: &str) -> Option<&atlas::config::ModelSpec> {
        self.worker.models.iter().find(|m| m.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACK: &str = r#"
version = "wa-demo-v1"

[[sections]]
id = "fees"
title = "Fees"
topic = "fees"
guidance = "Every office lists the per-apostille fee."

[[sections]]
id = "walkin"
title = "Walk-in hours"
topic = "walkin"
guidance = "List walk-in hours or say none."
"#;

    #[test]
    fn pack_loads_and_renders() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("general.toml");
        std::fs::write(&path, PACK).unwrap();
        let pack = TemplatePack::load(&path).unwrap();
        assert_eq!(pack.sections.len(), 2);
        assert!(pack.render().contains("[fees]"));
    }

    #[test]
    fn unknown_topic_fails() {
        let bad = PACK.replace("topic = \"fees\"", "topic = \"vibes\"");
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bad.toml");
        std::fs::write(&path, bad).unwrap();
        assert!(TemplatePack::load(&path).is_err());
    }
}
