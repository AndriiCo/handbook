//! Authoritative server-side submission validator (P2-F3).
//!
//! One rule source runs twice: the SPA ports these checks to TS for
//! instant feedback, and this module is the authority — no submission
//! persists while invalid. Rejection is per-field
//! `{key, code, message}`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::template::{FieldDef, FieldType, FormTemplate};

/// One field failure: `{key, code, message}` (P2-F3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldError {
    pub key: String,
    pub code: String,
    pub message: String,
}

impl FieldError {
    fn new(key: &str, code: &str, message: String) -> Self {
        Self {
            key: key.to_string(),
            code: code.to_string(),
            message,
        }
    }
}

/// Validate customer values against the template schema.
///
/// Returns the per-field errors; an empty vec means the submission may
/// persist. Unknown keys are ignored (the SPA may send extra draft
/// state; the server stores only schema keys).
pub fn validate_submission(
    template: &FormTemplate,
    values: &BTreeMap<String, Value>,
) -> Vec<FieldError> {
    let mut errors = Vec::new();
    for field in &template.fields {
        let value = values.get(&field.key);
        if is_blank(value) {
            if field.required {
                errors.push(FieldError::new(
                    &field.key,
                    "required",
                    format!("{} is required", field.label),
                ));
            }
            continue;
        }
        let Some(value) = value else { continue };
        check_max_len(field, value, &mut errors);
        check_pattern(field, value, &mut errors);
        check_type(field, value, &mut errors);
    }
    errors
}

fn is_blank(value: Option<&Value>) -> bool {
    match value {
        None => true,
        Some(Value::Null) => true,
        Some(Value::String(s)) => s.trim().is_empty(),
        Some(Value::Array(a)) => a.is_empty(),
        Some(Value::Object(o)) => o.values().all(|v| is_blank(Some(v))),
        Some(_) => false,
    }
}

fn check_max_len(field: &FieldDef, value: &Value, errors: &mut Vec<FieldError>) {
    let Some(max) = field.max_len else { return };
    if let Value::String(s) = value {
        let len = s.chars().count();
        if len > max {
            errors.push(FieldError::new(
                &field.key,
                "max_len",
                format!("{} must be at most {max} characters", field.label),
            ));
        }
    }
}

fn check_pattern(field: &FieldDef, value: &Value, errors: &mut Vec<FieldError>) {
    let Some(pattern) = &field.pattern else {
        return;
    };
    let Value::String(s) = value else { return };
    // Full-match semantics: `zip5` means exactly five digits, not "contains".
    let full = format!("^(?:{pattern})$");
    let matches = regex::Regex::new(&full).is_ok_and(|re| re.is_match(s));
    if !matches {
        errors.push(FieldError::new(
            &field.key,
            "pattern",
            format!("{} has an invalid format", field.label),
        ));
    }
}

fn check_type(field: &FieldDef, value: &Value, errors: &mut Vec<FieldError>) {
    match field.field_type {
        FieldType::Text | FieldType::Textarea => {
            if !matches!(value, Value::String(_)) {
                errors.push(FieldError::new(
                    &field.key,
                    "type",
                    format!("{} must be text", field.label),
                ));
            }
        }
        FieldType::Email => {
            let ok = matches!(value, Value::String(s) if is_email(s));
            if !ok {
                errors.push(FieldError::new(
                    &field.key,
                    "email",
                    format!("{} must be a valid email address", field.label),
                ));
            }
        }
        FieldType::Tel => {
            let ok = matches!(value, Value::String(s) if is_phone(s));
            if !ok {
                errors.push(FieldError::new(
                    &field.key,
                    "tel",
                    format!("{} must be a valid phone number", field.label),
                ));
            }
        }
        FieldType::Date => {
            let ok = matches!(value, Value::String(s) if is_date(s));
            if !ok {
                errors.push(FieldError::new(
                    &field.key,
                    "date",
                    format!("{} must be a date like 2026-11-04", field.label),
                ));
            }
        }
        FieldType::Number => {
            let ok = match value {
                Value::Number(_) => true,
                Value::String(s) => s.trim().parse::<f64>().is_ok(),
                _ => false,
            };
            if !ok {
                errors.push(FieldError::new(
                    &field.key,
                    "number",
                    format!("{} must be a number", field.label),
                ));
            }
        }
        FieldType::Money => {
            let ok = match value {
                Value::String(s) => parse_money_cents(s).is_some(),
                Value::Number(n) => n.as_i64().is_some_and(|c| c >= 0),
                _ => false,
            };
            if !ok {
                errors.push(FieldError::new(
                    &field.key,
                    "money",
                    format!("{} must be an amount like 25.00", field.label),
                ));
            }
        }
        FieldType::Select | FieldType::Radio => {
            let ok = matches!(value, Value::String(s) if field.options.iter().any(|o| o == s));
            if !ok {
                errors.push(FieldError::new(
                    &field.key,
                    "option",
                    format!("{} must be one of the listed options", field.label),
                ));
            }
        }
        FieldType::Checkbox => {
            if !matches!(value, Value::Bool(_)) {
                errors.push(FieldError::new(
                    &field.key,
                    "type",
                    format!("{} must be checked or unchecked", field.label),
                ));
            }
        }
        FieldType::AddressBlock => {
            if let Some(missing) = missing_address_part(value) {
                errors.push(FieldError::new(
                    &field.key,
                    "address",
                    format!("{} is missing {missing}", field.label),
                ));
            }
        }
    }
}

/// Parse `25`, `25.5`, or `25.00` into cents. Negative, empty, and
/// >2-decimal inputs are rejected.
pub fn parse_money_cents(input: &str) -> Option<i64> {
    let s = input.trim();
    if s.is_empty() || s.starts_with('-') || s.starts_with('+') {
        return None;
    }
    let (dollars, cents) = match s.split_once('.') {
        None => (s, "00"),
        Some((d, c)) => {
            if c.len() > 2 || c.is_empty() || !c.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (d, c)
        }
    };
    if dollars.is_empty() || !dollars.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let cents = if cents.len() == 1 {
        format!("{cents}0")
    } else {
        cents.to_string()
    };
    let total = dollars
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(cents.parse::<i64>().ok()?)?;
    Some(total)
}

fn is_email(s: &str) -> bool {
    // Small and strict enough for the server gate: one @, a dot in the
    // domain, no spaces. Full RFC parsing is out of scope.
    if s.bytes().any(|b| b.is_ascii_whitespace()) {
        return false;
    }
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    !local.is_empty() && !domain.starts_with('.') && domain.contains('.') && !domain.ends_with('.')
}

fn is_phone(s: &str) -> bool {
    let digits = s.bytes().filter(u8::is_ascii_digit).count();
    digits >= 7
        && s.bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b' ' | b'-' | b'(' | b')' | b'+' | b'.'))
}

fn is_date(s: &str) -> bool {
    // Strict YYYY-MM-DD including leap years; no timezone, no time.
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return false;
    }
    let (Some(y), Some(m), Some(d)) = (
        parts[0].parse::<u32>().ok(),
        parts[1].parse::<u32>().ok(),
        parts[2].parse::<u32>().ok(),
    ) else {
        return false;
    };
    if parts[0].len() != 4 || parts[1].len() != 2 || parts[2].len() != 2 {
        return false;
    }
    if m == 0 || m > 12 || d == 0 {
        return false;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let max = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if leap {
                29
            } else {
                28
            }
        }
    };
    d <= max
}

fn missing_address_part(value: &Value) -> Option<&'static str> {
    let obj = value.as_object()?;
    for part in ["name", "street", "city", "state", "zip"] {
        let filled = obj.get(part).is_some_and(|v| !is_blank(Some(v)));
        if !filled {
            return Some(part);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::PdfRect;
    use serde_json::json;

    fn rect() -> PdfRect {
        PdfRect {
            page: 1,
            x_mm: 20.0,
            y_mm: 60.0,
            w_mm: 170.0,
            h_mm: 8.0,
            font_size_pt: 11.0,
            multiline: false,
        }
    }

    fn field(key: &str, field_type: FieldType) -> FieldDef {
        FieldDef {
            key: key.to_string(),
            label: key.to_string(),
            help: None,
            field_type,
            required: true,
            max_len: None,
            pattern: None,
            options: Vec::new(),
            prefill_topic: None,
            pdf: rect(),
        }
    }

    fn template_with(fields: Vec<FieldDef>) -> FormTemplate {
        let mut template = crate::template::tests::sample_template();
        template.fields = fields;
        template
    }

    fn values(pairs: &[(&str, Value)]) -> BTreeMap<String, Value> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect()
    }

    #[test]
    fn required_and_max_len() {
        let mut name = field("name", FieldType::Text);
        name.max_len = Some(3);
        let template = template_with(vec![name]);
        assert_eq!(
            validate_submission(&template, &values(&[]))[0].code,
            "required"
        );
        let errors = validate_submission(&template, &values(&[("name", json!("abcd"))]));
        assert_eq!(errors[0].code, "max_len");
        assert!(validate_submission(&template, &values(&[("name", json!("abc"))])).is_empty());
    }

    #[test]
    fn pattern_is_full_match() {
        let mut zip = field("zip", FieldType::Text);
        zip.pattern = Some(r"\d{5}".to_string());
        let template = template_with(vec![zip]);
        assert!(validate_submission(&template, &values(&[("zip", json!("90210"))])).is_empty());
        let errors = validate_submission(&template, &values(&[("zip", json!("x90210x"))]));
        assert_eq!(errors[0].code, "pattern");
    }

    #[test]
    fn typed_checks() {
        let mut service = field("service", FieldType::Select);
        service.options = vec!["a".to_string(), "b".to_string()];
        let template = template_with(vec![
            field("email", FieldType::Email),
            field("tel", FieldType::Tel),
            field("date", FieldType::Date),
            field("amount", FieldType::Money),
            field("agree", FieldType::Checkbox),
            service,
        ]);
        let good = values(&[
            ("email", json!("a@b.co")),
            ("tel", json!("+1 (555) 123-4567")),
            ("date", json!("2024-02-29")),
            ("amount", json!("25.00")),
            ("agree", json!(true)),
            ("service", json!("a")),
        ]);
        assert!(validate_submission(&template, &good).is_empty());

        for (key, bad, code) in [
            ("email", json!("nope"), "email"),
            ("email", json!("a@b"), "email"),
            ("tel", json!("123"), "tel"),
            ("date", json!("2023-02-29"), "date"),
            ("date", json!("11/04/2026"), "date"),
            ("amount", json!("25.999"), "money"),
            ("amount", json!("-5"), "money"),
            ("agree", json!("yes"), "type"),
            ("service", json!("zzz"), "option"),
        ] {
            let mut attempt = good.clone();
            attempt.insert(key.to_string(), bad);
            let errors = validate_submission(&template, &attempt);
            assert_eq!(errors.len(), 1, "input {key}");
            assert_eq!(errors[0].code, code, "input {key}");
            assert_eq!(errors[0].key, key);
        }
    }

    #[test]
    fn money_parsing() {
        assert_eq!(parse_money_cents("25"), Some(2500));
        assert_eq!(parse_money_cents("25.5"), Some(2550));
        assert_eq!(parse_money_cents("25.00"), Some(2500));
        assert_eq!(parse_money_cents("0.99"), Some(99));
        for bad in ["", "-1", "+1", "1.234", "1.", ".5", "x", "1,000"] {
            assert_eq!(parse_money_cents(bad), None, "input {bad}");
        }
    }

    #[test]
    fn address_block_completeness() {
        let template = template_with(vec![field("mail_to", FieldType::AddressBlock)]);
        let full = json!({
            "name": "N", "street": "S", "city": "C", "state": "WA", "zip": "00000",
        });
        assert!(validate_submission(&template, &values(&[("mail_to", full)])).is_empty());
        let short = json!({"name": "N", "street": "S"});
        let errors = validate_submission(&template, &values(&[("mail_to", short)]));
        assert_eq!(errors[0].code, "address");
    }
}
