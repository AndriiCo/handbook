//! Draft + refine: template + findings → instruction sections.
//!
//! Two structured calls. Draft renders every template section from
//! the general requirements + the authority's live facts + the
//! findings brief. Refine applies special findings as explicit
//! overrides (even single-authority quirks), each with a reason.
//! Context is budgeted before sending (estimate, then live
//! `input_tokens` unless dry); over budget fails, never truncates.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use atlas::chunk::estimate_tokens;
use atlas::model::{ChatRequest, JsonSchema, ModelPort};

use crate::error::{Error, Result};
use crate::template::TemplatePack;
use crate::trigger::Trigger;

/// One live (accepted) fact for diffing.
#[derive(Debug, Clone)]
pub struct LiveFact {
    pub id: i64,
    pub topic: String,
    pub statement: String,
}

/// One drafted section.
#[derive(Debug, Clone)]
pub struct SectionDraft {
    pub id: String,
    pub topic: String,
    pub heading: String,
    pub body: String,
    pub statements: Vec<StatementDraft>,
    pub citations: Vec<Citation>,
}

/// One `field: value` statement inside a section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatementDraft {
    pub field: String,
    pub value: String,
}

/// Citation into a trigger sidecar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    pub capture: i64,
    pub start: u32,
    pub end: u32,
}

/// One applied refinement.
#[derive(Debug, Clone)]
pub struct Override {
    pub section: String,
    pub refined_body: String,
    /// Full replacement statement set (empty = keep the draft's).
    pub refined_statements: Vec<StatementDraft>,
    pub reason: String,
}

/// Draft output: final sections + the override trail.
#[derive(Debug)]
pub struct DraftOutput {
    pub sections: Vec<SectionDraft>,
    pub overrides: Vec<Override>,
}

/// Draft + refine one authority's instructions.
#[allow(clippy::too_many_arguments)]
pub fn draft_instructions<P: ModelPort>(
    port: &P,
    model_id: &str,
    context_window: u32,
    max_output_tokens: u32,
    template: &TemplatePack,
    live_facts: &[LiveFact],
    triggers: &[Trigger],
    dry: bool,
) -> Result<DraftOutput> {
    let context = assemble_context(template, live_facts, triggers);
    let usable = context_window.saturating_sub(max_output_tokens + 4096);
    let est = estimate_tokens(&context);
    if est > usable {
        return Err(Error::Budget(format!(
            "context estimated {est} tokens exceeds usable window {usable} — narrow triggers or summarize more"
        )));
    }
    if !dry {
        let measured =
            port.count_tokens(model_id, &json!([{"role": "user", "content": context}]))?;
        if measured > usable {
            return Err(Error::Budget(format!(
                "context measured {measured} tokens exceeds usable window {usable}"
            )));
        }
    }
    let sections_schema = JsonSchema {
        name: "InstructionSections".to_string(),
        schema: sections_schema(),
        strict: false,
    };
    let draft = port.complete_chat(&ChatRequest {
        model: model_id.to_string(),
        instructions: format!(
            "Draft per-authority instructions. Follow the general template; ground every \
             statement in the live facts and findings; cite trigger captures with byte spans. \
             Return JSON only.\n\nTEMPLATE:\n{}",
            template.render()
        ),
        user_text: context.clone(),
        schema: Some(sections_schema),
        max_output_tokens,
        temperature: 0.1,
        reasoning_effort: None,
        cache_key: Some(format!("instruct-draft-{}", template.version)),
    })?;
    let mut sections = parse_sections(&draft.text)?;
    validate_sections(template, triggers, &sections)?;
    let refine_schema = JsonSchema {
        name: "InstructionOverrides".to_string(),
        schema: overrides_schema(),
        strict: false,
    };
    let refine = port.complete_chat(&ChatRequest {
        model: model_id.to_string(),
        instructions: format!(
            "Refine the drafted sections against special findings — quirks that apply to \
             this authority even when they contradict the general template. Return one \
             override per affected section with a reason; omit unaffected sections. \
             Return JSON only.\n\nTEMPLATE:\n{}",
            template.render()
        ),
        user_text: format!(
            "SECTIONS:\n{}\n\nFINDINGS:\n{}",
            serde_json::to_string_pretty(&sections_json(&sections)).expect("sections serialize"),
            findings_text(triggers),
        ),
        schema: Some(refine_schema),
        max_output_tokens,
        temperature: 0.1,
        reasoning_effort: None,
        cache_key: None,
    })?;
    let overrides = parse_overrides(&refine.text)?;
    apply_overrides(template, &mut sections, overrides)
}

/// Assemble the draft context: live facts + findings.
fn assemble_context(
    template: &TemplatePack,
    live_facts: &[LiveFact],
    triggers: &[Trigger],
) -> String {
    let mut out = format!("TEMPLATE VERSION: {}\n\nLIVE FACTS:\n", template.version);
    if live_facts.is_empty() {
        out.push_str("(none yet)\n");
    }
    for fact in live_facts {
        out.push_str(&format!(
            "#{} [{}] {}\n",
            fact.id, fact.topic, fact.statement
        ));
    }
    out.push_str("\nFINDINGS:\n");
    out.push_str(&findings_text(triggers));
    out
}

fn findings_text(triggers: &[Trigger]) -> String {
    triggers
        .iter()
        .map(|t| {
            format!(
                "[capture #{} kind={}{}]\n{}",
                t.capture_id,
                t.kind,
                if t.condensed { " condensed" } else { "" },
                t.text
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn sections_schema() -> Value {
    json!({
        "type": "object",
        "required": ["sections"],
        "properties": {
            "sections": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["id", "topic", "heading", "body", "statements", "citations"],
                    "properties": {
                        "id": {"type": "string"},
                        "topic": {"type": "string"},
                        "heading": {"type": "string"},
                        "body": {"type": "string"},
                        "statements": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["field", "value"],
                                "properties": {
                                    "field": {"type": "string"},
                                    "value": {"type": "string"},
                                },
                            },
                        },
                        "citations": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["capture", "start", "end"],
                                "properties": {
                                    "capture": {"type": "integer"},
                                    "start": {"type": "integer"},
                                    "end": {"type": "integer"},
                                },
                            },
                        },
                    },
                },
            },
        },
    })
}

fn overrides_schema() -> Value {
    json!({
        "type": "object",
        "required": ["overrides"],
        "properties": {
            "overrides": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["section", "refined_body", "refined_statements", "reason"],
                    "properties": {
                        "section": {"type": "string"},
                        "refined_body": {"type": "string"},
                        "refined_statements": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "required": ["field", "value"],
                                "properties": {
                                    "field": {"type": "string"},
                                    "value": {"type": "string"},
                                },
                            },
                        },
                        "reason": {"type": "string"},
                    },
                },
            },
        },
    })
}

fn sections_json(sections: &[SectionDraft]) -> Value {
    json!(
        sections
            .iter()
            .map(|s| json!({
                "id": s.id, "topic": s.topic, "heading": s.heading, "body": s.body,
            }))
            .collect::<Vec<_>>()
    )
}

fn parse_sections(text: &str) -> Result<Vec<SectionDraft>> {
    let value: Value = extract_json(text)?;
    let items = value
        .get("sections")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Validate("draft has no sections array".to_string()))?;
    let mut sections = Vec::with_capacity(items.len());
    for item in items {
        let get = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| Error::Validate(format!("section missing '{key}'")))
        };
        let mut statements = Vec::new();
        for stmt in item
            .get("statements")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            statements.push(StatementDraft {
                field: stmt
                    .get("field")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                value: stmt
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
        let mut citations = Vec::new();
        for cite in item
            .get("citations")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            citations.push(Citation {
                capture: cite.get("capture").and_then(Value::as_i64).unwrap_or(0),
                start: cite.get("start").and_then(Value::as_u64).unwrap_or(0) as u32,
                end: cite.get("end").and_then(Value::as_u64).unwrap_or(0) as u32,
            });
        }
        sections.push(SectionDraft {
            id: get("id")?,
            topic: get("topic")?,
            heading: get("heading")?,
            body: get("body")?,
            statements,
            citations,
        });
    }
    Ok(sections)
}

fn parse_overrides(text: &str) -> Result<Vec<Override>> {
    let value: Value = extract_json(text)?;
    let items = value
        .get("overrides")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Validate("refine has no overrides array".to_string()))?;
    let mut overrides = Vec::with_capacity(items.len());
    for item in items {
        let get = |key: &str| {
            item.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .ok_or_else(|| Error::Validate(format!("override missing '{key}'")))
        };
        let mut refined_statements = Vec::new();
        for stmt in item
            .get("refined_statements")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Validate("override missing 'refined_statements'".to_string()))?
        {
            refined_statements.push(StatementDraft {
                field: stmt
                    .get("field")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                value: stmt
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            });
        }
        overrides.push(Override {
            section: get("section")?,
            refined_body: get("refined_body")?,
            refined_statements,
            reason: get("reason")?,
        });
    }
    Ok(overrides)
}

/// Parse model JSON: whole body, else the first `{…}` span.
fn extract_json(text: &str) -> Result<Value> {
    if let Ok(value) = serde_json::from_str(text) {
        return Ok(value);
    }
    let (Some(start), Some(end)) = (text.find('{'), text.rfind('}')) else {
        return Err(Error::Validate("model output is not JSON".to_string()));
    };
    if start >= end {
        return Err(Error::Validate("model output is not JSON".to_string()));
    }
    serde_json::from_str(&text[start..=end])
        .map_err(|_| Error::Validate("model output is not JSON".to_string()))
}

/// Validate sections: ids ∈ template, topics match the pack
/// mapping, citations cite known triggers within bounds.
fn validate_sections(
    template: &TemplatePack,
    triggers: &[Trigger],
    sections: &[SectionDraft],
) -> Result<()> {
    let topics: HashMap<&str, &str> = template
        .sections
        .iter()
        .map(|s| (s.id.as_str(), s.topic.as_str()))
        .collect();
    let texts: HashMap<i64, &str> = triggers
        .iter()
        .map(|t| (t.capture_id, t.text.as_str()))
        .collect();
    let mut seen = HashSet::new();
    for section in sections {
        if !seen.insert(section.id.as_str()) {
            return Err(Error::Validate(format!(
                "duplicate section '{}'",
                section.id
            )));
        }
        let Some(expect_topic) = topics.get(section.id.as_str()) else {
            return Err(Error::Validate(format!(
                "section '{}' is not in the template",
                section.id
            )));
        };
        if section.topic != *expect_topic {
            return Err(Error::Validate(format!(
                "section '{}': topic '{}' contradicts pack mapping '{}'",
                section.id, section.topic, expect_topic
            )));
        }
        if section.body.trim().is_empty() {
            return Err(Error::Validate(format!(
                "section '{}' has an empty body",
                section.id
            )));
        }
        for citation in &section.citations {
            let Some(text) = texts.get(&citation.capture) else {
                return Err(Error::Validate(format!(
                    "section '{}' cites unknown capture #{}",
                    section.id, citation.capture
                )));
            };
            if citation.start >= citation.end
                || citation.end as usize > text.len()
                || text
                    .get(citation.start as usize..citation.end as usize)
                    .is_none()
            {
                return Err(Error::Validate(format!(
                    "section '{}' cites out-of-bounds span in capture #{}",
                    section.id, citation.capture
                )));
            }
        }
    }
    Ok(())
}

/// Apply overrides (unknown sections are rejected, never silently
/// dropped — the model must not invent sections in refine).
fn apply_overrides(
    template: &TemplatePack,
    sections: &mut [SectionDraft],
    overrides: Vec<Override>,
) -> Result<DraftOutput> {
    let ids: HashSet<&str> = template.sections.iter().map(|s| s.id.as_str()).collect();
    for over in &overrides {
        if !ids.contains(over.section.as_str()) {
            return Err(Error::Validate(format!(
                "override targets unknown section '{}'",
                over.section
            )));
        }
        if over.refined_body.trim().is_empty() {
            return Err(Error::Validate(format!(
                "override for '{}' has an empty body",
                over.section
            )));
        }
    }
    for section in sections.iter_mut() {
        if let Some(over) = overrides.iter().find(|o| o.section == section.id) {
            section.body = over.refined_body.clone();
            if !over.refined_statements.is_empty() {
                section.statements = over.refined_statements.clone();
            }
        }
    }
    Ok(DraftOutput {
        sections: sections.to_vec(),
        overrides,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trigger() -> Trigger {
        Trigger {
            capture_id: 7,
            kind: "email".to_string(),
            text: "The fee is $30.".to_string(),
            condensed: false,
        }
    }

    #[test]
    fn validation_catches_bad_sections() {
        let pack = TemplatePack {
            version: "v".to_string(),
            sections: vec![crate::template::TemplateSection {
                id: "fees".to_string(),
                title: "Fees".to_string(),
                topic: "fees".to_string(),
                guidance: "g".to_string(),
            }],
            worker: Default::default(),
        };
        let good = SectionDraft {
            id: "fees".to_string(),
            topic: "fees".to_string(),
            heading: "Fees".to_string(),
            body: "Fee is $30.".to_string(),
            statements: vec![],
            citations: vec![Citation {
                capture: 7,
                start: 0,
                end: 4,
            }],
        };
        assert!(validate_sections(&pack, &[trigger()], std::slice::from_ref(&good)).is_ok());
        let mut bad_topic = good.clone();
        bad_topic.topic = "walkin".to_string();
        assert!(validate_sections(&pack, &[trigger()], &[bad_topic]).is_err());
        let mut bad_span = good.clone();
        bad_span.citations = vec![Citation {
            capture: 7,
            start: 0,
            end: 400,
        }];
        assert!(validate_sections(&pack, &[trigger()], &[bad_span]).is_err());
        let mut bad_cap = good.clone();
        bad_cap.citations = vec![Citation {
            capture: 9,
            start: 0,
            end: 1,
        }];
        assert!(validate_sections(&pack, &[trigger()], &[bad_cap]).is_err());
    }
}
