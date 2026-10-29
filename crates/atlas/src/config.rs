//! Task-pack configuration (TOML).
//!
//! A pack is data: sources, models, effort, and tasks. Prompts and
//! schemas live in the pack (or files it points at) — the harness
//! never invents them. [`AtlasConfig::load`] parses, resolves
//! relative schema paths against the pack's directory, and
//! validates cross-references.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Top-level pack file.
#[derive(Debug, Clone, Deserialize)]
pub struct AtlasConfig {
    #[serde(default)]
    pub run: RunConfig,
    #[serde(default)]
    pub models: Vec<ModelSpec>,
    #[serde(default)]
    pub sources: Vec<SourceSpec>,
    #[serde(default)]
    pub tasks: Vec<TaskSpec>,
}

/// `[run]`: output dir + default effort.
#[derive(Debug, Clone, Deserialize)]
pub struct RunConfig {
    #[serde(default = "default_out")]
    pub out: String,
    #[serde(default)]
    pub effort: Effort,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            out: default_out(),
            effort: Effort::default(),
        }
    }
}

fn default_out() -> String {
    "out/atlas".to_string()
}

/// Standard vs Contributor mode. Contributor runs the same weights
/// at discounted prices in exchange for training on your content —
/// tasks must opt in explicitly (see [`TaskSpec::allow_contributor`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum UseMode {
    Standard,
    Contributor,
}

/// One priced model entry. Token windows, rate limits, and prices
/// come from the pack (docs drift; code must not pin them).
#[derive(Debug, Clone, Deserialize)]
pub struct ModelSpec {
    /// Task-local alias (`model = "spark"`).
    pub name: String,
    /// API model id (`muse-spark-1.3`, `…-contributor`, …).
    pub id: String,
    #[serde(default = "standard_mode")]
    pub mode: UseMode,
    pub context_window: u32,
    #[serde(default = "default_max_output")]
    pub max_output_tokens: u32,
    #[serde(default)]
    pub rpm: u32,
    #[serde(default)]
    pub tpm: u32,
    /// USD per 1M tokens (0 = unknown; ledger records 0 cost).
    #[serde(default)]
    pub input_per_1m: f64,
    #[serde(default)]
    pub output_per_1m: f64,
}

fn standard_mode() -> UseMode {
    UseMode::Standard
}

fn default_max_output() -> u32 {
    4096
}

/// Effort preset: how hard the harness works per task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    #[default]
    Standard,
    High,
    Max,
}

impl Effort {
    /// Parse a CLI `--effort` value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Effort::Low),
            "standard" => Some(Effort::Standard),
            "high" => Some(Effort::High),
            "max" => Some(Effort::Max),
            _ => None,
        }
    }

    /// Resolve preset numbers (tasks may override chunk/verify).
    pub fn preset(self) -> EffortPreset {
        match self {
            Effort::Low => EffortPreset {
                chunk_tokens: 8_000,
                overlap_tokens: 400,
                summary_levels: 1,
                verify: Verify::None,
                batch_threshold: 200,
            },
            Effort::Standard => EffortPreset {
                chunk_tokens: 16_000,
                overlap_tokens: 800,
                summary_levels: 2,
                verify: Verify::None,
                batch_threshold: 100,
            },
            Effort::High => EffortPreset {
                chunk_tokens: 32_000,
                overlap_tokens: 1_600,
                summary_levels: 2,
                verify: Verify::Same,
                batch_threshold: 50,
            },
            Effort::Max => EffortPreset {
                chunk_tokens: 64_000,
                overlap_tokens: 3_200,
                summary_levels: 3,
                verify: Verify::Same,
                batch_threshold: 20,
            },
        }
    }
}

/// Numbers behind an [`Effort`] preset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffortPreset {
    pub chunk_tokens: u32,
    pub overlap_tokens: u32,
    pub summary_levels: u8,
    pub verify: Verify,
    /// `protocol = auto` switches to batch past this many calls.
    pub batch_threshold: usize,
}

/// Second-pass verification policy.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Verify {
    None,
    /// Re-check with the same model (cheap contradiction pass).
    Same,
    /// Re-check with a named (usually stronger) model.
    Model(String),
}

impl Verify {
    /// Parse a pack `verify` value: `none`, `self`, or a model name.
    pub fn parse(value: &str) -> Self {
        match value {
            "none" => Verify::None,
            "self" => Verify::Same,
            name => Verify::Model(name.to_string()),
        }
    }
}

/// Fast (sync chat) vs batch (background jobs) vs auto-select.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    #[default]
    Fast,
    Batch,
    Auto,
}

impl Protocol {
    /// Parse a CLI `--protocol` value.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "fast" => Some(Protocol::Fast),
            "batch" => Some(Protocol::Batch),
            "auto" => Some(Protocol::Auto),
            _ => None,
        }
    }
}

/// Input media kind. Text-like kinds extract locally; `Pdf` and
/// `Audio`/`Image` may also ride to the model natively (see
/// [`crate::extract`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Text,
    Html,
    Pdf,
    Svg,
    Audio,
    Image,
}

/// Re-fetch policy for URL sources.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Refresh {
    Always,
    Daily,
    #[default]
    Weekly,
    Never,
}

/// One named input: URL (`https://…`) or pack-relative file path.
#[derive(Debug, Clone, Deserialize)]
pub struct SourceSpec {
    pub name: String,
    pub url: String,
    pub kind: SourceKind,
    #[serde(default)]
    pub refresh: Refresh,
}

/// One unit of model work.
#[derive(Debug, Clone, Deserialize)]
pub struct TaskSpec {
    pub name: String,
    /// Model alias from `[[models]]`.
    pub model: String,
    #[serde(default)]
    pub protocol: Protocol,
    /// Overrides `[run] effort` when set.
    pub effort: Option<Effort>,
    /// Pack-owned prompt (developer instructions).
    #[serde(default)]
    pub instructions: String,
    /// Path (pack-relative) to the JSON output schema.
    #[serde(default)]
    pub schema: Option<String>,
    /// `none` | `self` | model name (default: effort preset).
    #[serde(default)]
    pub verify: Option<String>,
    /// Required when the model runs Contributor mode.
    #[serde(default)]
    pub allow_contributor: bool,
    /// Source names, or `["*"]` for all.
    #[serde(default = "all_sources")]
    pub inputs: Vec<String>,
    /// Debug seam: cap documents per run (0 = no cap).
    #[serde(default)]
    pub limit: usize,
    /// Override the effort chunk size (tokens, estimator units).
    #[serde(default)]
    pub chunk_tokens: Option<u32>,
}

fn all_sources() -> Vec<String> {
    vec!["*".to_string()]
}

impl AtlasConfig {
    /// Load + validate a pack file.
    pub fn load(path: &Path) -> Result<(Self, PathBuf)> {
        let text = std::fs::read_to_string(path).map_err(Error::Io)?;
        let mut cfg: AtlasConfig =
            toml::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        let dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        cfg.resolve_schema_paths(&dir)?;
        cfg.validate()?;
        Ok((cfg, dir))
    }

    /// Parse without file I/O (tests, `--dry-run` previews).
    pub fn from_toml_str(text: &str) -> Result<Self> {
        let cfg: AtlasConfig = toml::from_str(text).map_err(|e| Error::Config(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    fn resolve_schema_paths(&mut self, dir: &Path) -> Result<()> {
        for task in &mut self.tasks {
            if let Some(schema) = task.schema.take() {
                let candidate = dir.join(&schema);
                if candidate.is_file() {
                    task.schema = Some(candidate.to_string_lossy().into_owned());
                } else if Path::new(&schema).is_file() {
                    task.schema = Some(schema);
                } else {
                    return Err(Error::Config(format!(
                        "task '{}': schema not found: {schema}",
                        task.name
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        let models: HashMap<&str, &ModelSpec> =
            self.models.iter().map(|m| (m.name.as_str(), m)).collect();
        let sources: HashMap<&str, &SourceSpec> =
            self.sources.iter().map(|s| (s.name.as_str(), s)).collect();
        for task in &self.tasks {
            let model = models.get(task.model.as_str()).ok_or_else(|| {
                Error::Config(format!(
                    "task '{}': unknown model '{}'",
                    task.name, task.model
                ))
            })?;
            if model.mode == UseMode::Contributor && !task.allow_contributor {
                return Err(Error::Config(format!(
                    "task '{}': model '{}' is Contributor mode; set allow_contributor = true to opt in",
                    task.name, task.model
                )));
            }
            if let Some(verify) = task.verify.as_deref()
                && let Verify::Model(name) = Verify::parse(verify)
                && !models.contains_key(name.as_str())
            {
                return Err(Error::Config(format!(
                    "task '{}': unknown verify model '{name}'",
                    task.name
                )));
            }
            for input in &task.inputs {
                if input != "*" && !sources.contains_key(input.as_str()) {
                    return Err(Error::Config(format!(
                        "task '{}': unknown source '{input}'",
                        task.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// Look up a model by task alias.
    pub fn model(&self, name: &str) -> Option<&ModelSpec> {
        self.models.iter().find(|m| m.name == name)
    }

    /// Resolve a task's input sources (`*` = all).
    pub fn task_sources<'a>(&'a self, task: &'a TaskSpec) -> Vec<&'a SourceSpec> {
        if task.inputs.iter().any(|i| i == "*") {
            return self.sources.iter().collect();
        }
        task.inputs
            .iter()
            .filter_map(|name| self.sources.iter().find(|s| &s.name == name))
            .collect()
    }

    /// Resolve a task's effort (task override or run default).
    pub fn task_effort(&self, task: &TaskSpec) -> EffortPreset {
        task.effort.unwrap_or(self.run.effort).preset()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACK: &str = r#"
[run]
out = "out/x"
effort = "high"

[[models]]
name = "spark"
id = "muse-spark-1.3"
mode = "standard"
context_window = 1048576
rpm = 3000
tpm = 4000000

[[models]]
name = "cheap"
id = "muse-spark-1.3-contributor"
mode = "contributor"
context_window = 1048576

[[sources]]
name = "status"
url = "https://example.test/status"
kind = "html"

[[tasks]]
name = "scan"
model = "spark"
protocol = "auto"
instructions = "Extract rows."
inputs = ["status"]
"#;

    #[test]
    fn pack_parses_and_validates() {
        let cfg = AtlasConfig::from_toml_str(PACK).unwrap();
        assert_eq!(cfg.run.out, "out/x");
        assert_eq!(cfg.run.effort, Effort::High);
        assert_eq!(cfg.model("spark").unwrap().id, "muse-spark-1.3");
        let task = &cfg.tasks[0];
        assert_eq!(cfg.task_sources(task).len(), 1);
        assert_eq!(cfg.task_effort(task).chunk_tokens, 32_000);
    }

    #[test]
    fn contributor_needs_opt_in() {
        let bad = PACK.replace("model = \"spark\"", "model = \"cheap\"");
        let err = AtlasConfig::from_toml_str(&bad).unwrap_err();
        assert!(format!("{err}").contains("allow_contributor"), "{err}");
        let ok = bad.replace(
            "inputs = [\"status\"]",
            "inputs = [\"status\"]\nallow_contributor = true",
        );
        assert!(AtlasConfig::from_toml_str(&ok).is_ok());
    }

    #[test]
    fn unknown_refs_fail() {
        let bad = PACK.replace("model = \"spark\"", "model = \"nope\"");
        assert!(AtlasConfig::from_toml_str(&bad).is_err());
        let bad = PACK.replace("inputs = [\"status\"]", "inputs = [\"nope\"]");
        assert!(AtlasConfig::from_toml_str(&bad).is_err());
    }

    #[test]
    fn effort_and_protocol_parse() {
        assert_eq!(Effort::parse("max"), Some(Effort::Max));
        assert_eq!(Effort::parse("turbo"), None);
        assert_eq!(Protocol::parse("batch"), Some(Protocol::Batch));
        assert_eq!(Protocol::parse("pigeon"), None);
        assert_eq!(Verify::parse("self"), Verify::Same);
        assert_eq!(Verify::parse("wonder"), Verify::Model("wonder".to_string()));
    }
}
