//! Instruction rendering: template + drafted sections → markdown.
//!
//! Pack order wins; sections without a draft render an explicit
//! "(no change proposed this run)" marker rather than invented
//! prose. Sections with proposals are flagged pending review.

use std::collections::HashSet;

use crate::draft::SectionDraft;
use crate::template::TemplatePack;

/// Render instructions markdown for reviewers (task detail + demo).
pub fn render_instructions(
    authority_name: &str,
    template: &TemplatePack,
    sections: &[SectionDraft],
    pending: &HashSet<&str>,
) -> String {
    let mut out = format!("# Instructions — {authority_name}\n\n");
    out.push_str(&format!("<!-- template {} -->\n\n", template.version));
    for expected in &template.sections {
        let draft = sections.iter().find(|s| s.id == expected.id);
        match draft {
            Some(section) => {
                let flag = if pending.contains(section.id.as_str()) {
                    " (pending review)"
                } else {
                    ""
                };
                out.push_str(&format!(
                    "## {}{flag}\n\n{}\n\n",
                    section.heading, section.body
                ));
            }
            None => {
                out.push_str(&format!(
                    "## {}\n\n(no change proposed this run)\n\n",
                    expected.title
                ));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_order_and_pending_flags() {
        let pack = TemplatePack {
            version: "v1".to_string(),
            sections: vec![
                crate::template::TemplateSection {
                    id: "fees".to_string(),
                    title: "Fees".to_string(),
                    topic: "fees".to_string(),
                    guidance: String::new(),
                },
                crate::template::TemplateSection {
                    id: "hours".to_string(),
                    title: "Hours".to_string(),
                    topic: "hours".to_string(),
                    guidance: String::new(),
                },
            ],
            worker: Default::default(),
        };
        let sections = vec![SectionDraft {
            id: "fees".to_string(),
            topic: "fees".to_string(),
            heading: "Fees".to_string(),
            body: "Pay $30.".to_string(),
            statements: vec![],
            citations: vec![],
        }];
        let pending: HashSet<&str> = ["fees"].into_iter().collect();
        let md = render_instructions("Demo Office", &pack, &sections, &pending);
        assert!(md.contains("## Fees (pending review)"));
        assert!(md.contains("## Hours\n\n(no change proposed this run)"));
        assert!(md.find("Fees").unwrap() < md.find("Hours").unwrap());
    }
}
