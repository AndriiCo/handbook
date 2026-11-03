//! Official fillable PDFs (F1): the AcroForm field list, the text layer
//! a field label must be quoted from, and fill-and-flatten — values are
//! drawn into the page content and the form layer is removed, so the
//! printed page cannot be edited and looks the same in every viewer.
//!
//! Pure lopdf; no rendering engine. Text is drawn in Helvetica with
//! WinAnsi encoding (characters outside it print as `?`).

use std::collections::BTreeMap;

use lopdf::content::{Content, Operation};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};

/// One fillable field, as staff and the model see it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AcroField {
    /// Fully qualified name (`parent.child`), the fill key.
    pub name: String,
    /// `text | checkbox | radio | choice | signature | button`.
    pub kind: String,
    /// Choice options or checkbox/radio export values.
    pub options: Vec<String>,
    /// 1-based page of the first widget, when known.
    pub page: Option<u32>,
    /// Max length (`/MaxLen`), when set.
    pub max_len: Option<u32>,
}

/// Errors are short strings: PDFs come from official sites and the
/// message never echoes their bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcroError(pub String);

impl std::fmt::Display for AcroError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> AcroError + '_ {
    move |e| AcroError(format!("{what}: {e}"))
}

/// PDF text string → Rust string (UTF-16BE with BOM, else Latin-1).
fn text_of(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes(*c))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        bytes.iter().map(|&b| b as char).collect()
    }
}

fn deref<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Object> {
    match obj {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

fn dict_of<'a>(doc: &'a Document, obj: &'a Object) -> Option<&'a Dictionary> {
    deref(doc, obj)?.as_dict().ok()
}

fn name_of(obj: Option<&Object>) -> Option<String> {
    match obj? {
        Object::Name(n) => Some(String::from_utf8_lossy(n).to_string()),
        _ => None,
    }
}

/// Inheritable attribute lookup up the `/Parent` chain.
fn inherited<'a>(doc: &'a Document, mut d: &'a Dictionary, key: &[u8]) -> Option<&'a Object> {
    for _ in 0..16 {
        if let Ok(v) = d.get(key) {
            return Some(v);
        }
        d = dict_of(doc, d.get(b"Parent").ok()?)?;
    }
    None
}

/// The page number (1-based) each page object id sits at.
fn page_numbers(doc: &Document) -> BTreeMap<ObjectId, u32> {
    doc.get_pages().into_iter().map(|(n, id)| (id, n)).collect()
}

/// Terminal fields: (qualified name, field dict id or None, widgets).
fn walk(
    doc: &Document,
    obj: &Object,
    prefix: &str,
    out: &mut Vec<(String, Vec<ObjectId>, ObjectId)>,
    depth: u32,
) {
    if depth > 16 {
        return;
    }
    let Object::Reference(id) = obj else { return };
    let Some(d) = doc.get_dictionary(*id).ok() else {
        return;
    };
    let part = d.get(b"T").ok().and_then(|t| t.as_str().ok()).map(text_of);
    let name = match (&part, prefix.is_empty()) {
        (Some(p), true) => p.clone(),
        (Some(p), false) => format!("{prefix}.{p}"),
        (None, _) => prefix.to_string(),
    };
    let kids: Vec<Object> = d
        .get(b"Kids")
        .ok()
        .and_then(|k| deref(doc, k))
        .and_then(|k| k.as_array().ok())
        .cloned()
        .unwrap_or_default();
    // Kids that carry their own /T are fields; kids without are widgets.
    let (fields, widgets): (Vec<Object>, Vec<Object>) = kids
        .into_iter()
        .partition(|k| dict_of(doc, k).is_some_and(|kd| kd.has(b"T")));
    for f in &fields {
        walk(doc, f, &name, out, depth + 1);
    }
    if fields.is_empty() {
        let mut ids: Vec<ObjectId> = widgets
            .iter()
            .filter_map(|w| w.as_reference().ok())
            .collect();
        if ids.is_empty() && d.get(b"Rect").is_ok() {
            ids.push(*id); // merged field + widget
        }
        if !name.is_empty() {
            out.push((name, ids, *id));
        }
    }
}

fn load(bytes: &[u8]) -> Result<Document, AcroError> {
    Document::load_mem(bytes).map_err(err("not a readable PDF"))
}

/// Every fillable field of the PDF's AcroForm, in document order.
/// A PDF without an AcroForm yields an empty list (a flat form).
pub fn fields(bytes: &[u8]) -> Result<Vec<AcroField>, AcroError> {
    let doc = load(bytes)?;
    let pages = page_numbers(&doc);
    let Some(form) = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|f| dict_of(&doc, f))
    else {
        return Ok(Vec::new());
    };
    let roots: Vec<Object> = form
        .get(b"Fields")
        .ok()
        .and_then(|f| deref(&doc, f))
        .and_then(|f| f.as_array().ok())
        .cloned()
        .unwrap_or_default();
    let mut terminal = Vec::new();
    for r in &roots {
        walk(&doc, r, "", &mut terminal, 0);
    }
    let mut out = Vec::new();
    for (name, widgets, field_id) in terminal {
        let Ok(d) = doc.get_dictionary(field_id) else {
            continue;
        };
        let ft = name_of(inherited(&doc, d, b"FT")).unwrap_or_default();
        let ff = inherited(&doc, d, b"Ff")
            .and_then(|f| f.as_i64().ok())
            .unwrap_or(0);
        let kind = match ft.as_str() {
            "Tx" => "text",
            "Ch" => "choice",
            "Sig" => "signature",
            "Btn" if ff & (1 << 16) != 0 => "button",
            "Btn" if ff & (1 << 15) != 0 => "radio",
            "Btn" => "checkbox",
            _ => continue,
        };
        let mut options: Vec<String> = Vec::new();
        if kind == "choice"
            && let Some(opts) = inherited(&doc, d, b"Opt")
                .and_then(|o| deref(&doc, o))
                .and_then(|o| o.as_array().ok())
        {
            for o in opts {
                match deref(&doc, o) {
                    Some(Object::String(s, _)) => options.push(text_of(s)),
                    Some(Object::Array(pair)) => {
                        if let Some(Object::String(s, _)) = pair.get(1).or(pair.first()) {
                            options.push(text_of(s));
                        }
                    }
                    _ => {}
                }
            }
        }
        if matches!(kind, "checkbox" | "radio") {
            for w in &widgets {
                let on = doc
                    .get_dictionary(*w)
                    .ok()
                    .and_then(|wd| wd.get(b"AP").ok())
                    .and_then(|ap| dict_of(&doc, ap))
                    .and_then(|ap| ap.get(b"N").ok())
                    .and_then(|n| dict_of(&doc, n))
                    .map(|n| {
                        n.iter()
                            .map(|(k, _)| String::from_utf8_lossy(k).to_string())
                            .filter(|k| k != "Off")
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for v in on {
                    if !options.contains(&v) {
                        options.push(v);
                    }
                }
            }
        }
        let page = widgets.first().and_then(|w| {
            let wd = doc.get_dictionary(*w).ok()?;
            let p = wd.get(b"P").ok()?.as_reference().ok()?;
            pages.get(&p).copied()
        });
        let max_len = inherited(&doc, d, b"MaxLen")
            .and_then(|m| m.as_i64().ok())
            .and_then(|m| u32::try_from(m).ok());
        out.push(AcroField {
            name,
            kind: kind.to_string(),
            options,
            page,
            max_len,
        });
    }
    Ok(out)
}

/// The PDF's text layer (every page), for label grounding.
pub fn text_layer(bytes: &[u8]) -> Result<String, AcroError> {
    let doc = load(bytes)?;
    let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    doc.extract_text(&pages).map_err(err("text layer"))
}

/// WinAnsi-safe PDF string bytes for `s`.
fn win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|c| match c {
            '\u{20}'..='\u{7e}' => c as u8,
            '\u{a0}'..='\u{ff}' => c as u32 as u8,
            '\u{2019}' => b'\'',
            '\u{2013}' | '\u{2014}' => b'-',
            _ => b'?',
        })
        .collect()
}

fn rect_of(d: &Dictionary) -> Option<[f32; 4]> {
    let r = d.get(b"Rect").ok()?.as_array().ok()?;
    let n: Vec<f32> = r
        .iter()
        .filter_map(|v| {
            v.as_float()
                .ok()
                .or_else(|| v.as_i64().ok().map(|i| i as f32))
        })
        .collect();
    (n.len() == 4).then(|| {
        [
            n[0].min(n[2]),
            n[1].min(n[3]),
            n[0].max(n[2]),
            n[1].max(n[3]),
        ]
    })
}

/// Draw `values` (keyed by qualified field name) into the pages and
/// remove the form layer. Checkbox/radio values are drawn as `X` when
/// truthy (`yes`, `true`, `on`, `1`, or the export value). Unknown keys
/// are an error (never silently dropped); an empty value draws nothing.
pub fn fill_flatten(bytes: &[u8], values: &BTreeMap<String, String>) -> Result<Vec<u8>, AcroError> {
    let mut doc = load(bytes)?;
    let known = fields(bytes)?;
    if let Some(bad) = values.keys().find(|k| !known.iter().any(|f| &f.name == *k)) {
        return Err(AcroError(format!("unknown field {bad:?}")));
    }
    // Widget id → (field, page id) for every terminal field.
    let catalog_form = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .cloned();
    let roots: Vec<Object> = catalog_form
        .as_ref()
        .and_then(|f| dict_of(&doc, f))
        .and_then(|f| f.get(b"Fields").ok())
        .and_then(|f| deref(&doc, f))
        .and_then(|f| f.as_array().ok())
        .cloned()
        .unwrap_or_default();
    let mut terminal = Vec::new();
    for r in &roots {
        walk(&doc, r, "", &mut terminal, 0);
    }
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    let mut per_page: BTreeMap<ObjectId, Vec<Operation>> = BTreeMap::new();
    let mut widget_ids: Vec<ObjectId> = Vec::new();
    for (name, widgets, field_id) in &terminal {
        widget_ids.extend(widgets.iter().copied());
        let Some(value) = values.get(name).map(|v| v.trim()).filter(|v| !v.is_empty()) else {
            continue;
        };
        let field = known.iter().find(|f| &f.name == name).expect("known field");
        let fd = doc
            .get_dictionary(*field_id)
            .ok()
            .cloned()
            .unwrap_or_default();
        for w in widgets {
            let Ok(wd) = doc.get_dictionary(*w).cloned() else {
                continue;
            };
            let Some([x0, y0, x1, y1]) = rect_of(&wd) else {
                continue;
            };
            let Some(page) = wd
                .get(b"P")
                .ok()
                .and_then(|p| p.as_reference().ok())
                .or_else(|| {
                    doc.get_pages().values().copied().find(|pid| {
                        doc.get_dictionary(*pid)
                            .ok()
                            .and_then(|pd| pd.get(b"Annots").ok())
                            .and_then(|a| deref(&doc, a))
                            .and_then(|a| a.as_array().ok())
                            .is_some_and(|a| a.iter().any(|o| o.as_reference().ok() == Some(*w)))
                    })
                })
            else {
                continue;
            };
            let h = y1 - y0;
            let text: String = match field.kind.as_str() {
                "checkbox" | "radio" => {
                    let lower = value.to_lowercase();
                    let truthy = matches!(lower.as_str(), "yes" | "true" | "on" | "1" | "x")
                        || field.options.iter().any(|o| o == value);
                    if !truthy {
                        continue;
                    }
                    "X".to_string()
                }
                _ => value.to_string(),
            };
            let size = if matches!(field.kind.as_str(), "checkbox" | "radio") {
                (h * 0.8).clamp(6.0, 14.0)
            } else {
                inherited(&doc, &fd, b"DA")
                    .and_then(|da| da.as_str().ok())
                    .and_then(|da| {
                        let s = String::from_utf8_lossy(da);
                        let toks: Vec<&str> = s.split_whitespace().collect();
                        toks.iter()
                            .position(|t| *t == "Tf")
                            .and_then(|i| toks.get(i.wrapping_sub(1)))
                            .and_then(|v| v.parse::<f32>().ok())
                    })
                    .filter(|s| *s > 0.0)
                    .unwrap_or((h * 0.65).clamp(6.0, 11.0))
            };
            let ops = per_page.entry(page).or_default();
            ops.extend([
                Operation::new("q", vec![]),
                Operation::new("re", vec![x0.into(), y0.into(), (x1 - x0).into(), h.into()]),
                Operation::new("W", vec![]),
                Operation::new("n", vec![]),
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["HbFill".into(), size.into()]),
                Operation::new(
                    "Td",
                    vec![
                        (x0 + 2.0).into(),
                        (y0 + (h - size) / 2.0 + size * 0.2).into(),
                    ],
                ),
                Operation::new(
                    "Tj",
                    vec![Object::String(win_ansi(&text), StringFormat::Literal)],
                ),
                Operation::new("ET", vec![]),
                Operation::new("Q", vec![]),
            ]);
        }
    }
    for (page, ops) in per_page {
        let content = Content { operations: ops }
            .encode()
            .map_err(err("encode content"))?;
        let stream_id = doc.add_object(Stream::new(Dictionary::new(), content));
        // Append the drawing stream to the page's contents.
        let page_dict = doc.get_dictionary(page).map_err(err("page"))?.clone();
        let mut contents: Vec<Object> = match page_dict.get(b"Contents") {
            Ok(Object::Array(a)) => a.clone(),
            Ok(Object::Reference(r)) => match doc.get_object(*r) {
                Ok(Object::Array(a)) => a.clone(),
                _ => vec![Object::Reference(*r)],
            },
            _ => Vec::new(),
        };
        contents.push(Object::Reference(stream_id));
        // Fonts: resolve the page's resources, add our font.
        let mut resources = page_dict
            .get(b"Resources")
            .ok()
            .and_then(|r| dict_of(&doc, r))
            .cloned()
            .unwrap_or_default();
        let mut fonts = resources
            .get(b"Font")
            .ok()
            .and_then(|f| dict_of(&doc, f))
            .cloned()
            .unwrap_or_default();
        fonts.set("HbFill", Object::Reference(font_id));
        resources.set("Font", Object::Dictionary(fonts));
        let page_mut = doc.get_dictionary_mut(page).map_err(err("page"))?;
        page_mut.set("Contents", Object::Array(contents));
        page_mut.set("Resources", Object::Dictionary(resources));
    }
    // Flatten: drop every form widget from every page, then the form.
    for pid in doc.get_pages().values().copied().collect::<Vec<_>>() {
        let annots = doc
            .get_dictionary(pid)
            .ok()
            .and_then(|pd| pd.get(b"Annots").ok())
            .and_then(|a| deref(&doc, a))
            .and_then(|a| a.as_array().ok())
            .cloned();
        if let Some(annots) = annots {
            let kept: Vec<Object> = annots
                .into_iter()
                .filter(|o| {
                    o.as_reference()
                        .ok()
                        .is_none_or(|id| !widget_ids.contains(&id))
                })
                .collect();
            if let Ok(pd) = doc.get_dictionary_mut(pid) {
                pd.set("Annots", Object::Array(kept));
            }
        }
    }
    if let Ok(root) = doc.trailer.get(b"Root").and_then(|r| r.as_reference())
        && let Ok(cat) = doc.get_dictionary_mut(root)
    {
        cat.remove(b"AcroForm");
    }
    let mut out = Vec::new();
    doc.save_to(&mut out).map_err(err("write PDF"))?;
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A synthetic one-page AcroForm: a text field, a checkbox and a
    /// choice, with the label text on the page.
    pub fn sample_form() -> Vec<u8> {
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(
            dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"},
        );
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 720.into()]),
                Operation::new(
                    "Tj",
                    vec![Object::string_literal("SYNTHETIC Apostille Request Form")],
                ),
                Operation::new("Td", vec![0.into(), (-30).into()]),
                Operation::new(
                    "Tj",
                    vec![Object::string_literal("Name of person requesting")],
                ),
                Operation::new("Td", vec![0.into(), (-30).into()]),
                Operation::new("Tj", vec![Object::string_literal("Return by mail")]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.new_object_id();
        let text = doc.add_object(dictionary! {
            "FT" => "Tx", "T" => Object::string_literal("RequesterName"), "Type" => "Annot", "Subtype" => "Widget",
            "Rect" => vec![250.into(), 685.into(), 500.into(), 705.into()], "P" => Object::Reference(page_id),
            "DA" => Object::string_literal("/Helv 10 Tf 0 g"),
        });
        let on_ap = doc.add_object(Stream::new(dictionary! {}, b"".to_vec()));
        let check = doc.add_object(dictionary! {
            "FT" => "Btn", "T" => Object::string_literal("ReturnMail"), "Type" => "Annot", "Subtype" => "Widget",
            "Rect" => vec![250.into(), 655.into(), 262.into(), 667.into()], "P" => Object::Reference(page_id),
            "AP" => dictionary! {"N" => dictionary! {"Yes" => Object::Reference(on_ap), "Off" => Object::Reference(on_ap)}},
        });
        let choice = doc.add_object(dictionary! {
            "FT" => "Ch", "T" => Object::string_literal("Speed"), "Type" => "Annot", "Subtype" => "Widget",
            "Rect" => vec![250.into(), 625.into(), 400.into(), 640.into()], "P" => Object::Reference(page_id),
            "Opt" => vec![Object::string_literal("Standard"), Object::string_literal("Expedited")],
        });
        doc.objects.insert(page_id, Object::Dictionary(dictionary! {
            "Type" => "Page", "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => Object::Reference(content_id),
            "Resources" => dictionary! {"Font" => dictionary! {"F1" => Object::Reference(font)}},
            "Annots" => vec![Object::Reference(text), Object::Reference(check), Object::Reference(choice)],
        }));
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1,
            }),
        );
        let form = doc.add_object(dictionary! {
            "Fields" => vec![Object::Reference(text), Object::Reference(check), Object::Reference(choice)],
        });
        let catalog = doc.add_object(dictionary! {
            "Type" => "Catalog", "Pages" => Object::Reference(pages_id), "AcroForm" => Object::Reference(form),
        });
        doc.trailer.set("Root", Object::Reference(catalog));
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    #[test]
    fn lists_fields_with_kinds_options_and_pages() {
        let f = fields(&sample_form()).unwrap();
        let names: Vec<(&str, &str)> = f
            .iter()
            .map(|x| (x.name.as_str(), x.kind.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("RequesterName", "text"),
                ("ReturnMail", "checkbox"),
                ("Speed", "choice")
            ]
        );
        assert_eq!(f[1].options, ["Yes"]);
        assert_eq!(f[2].options, ["Standard", "Expedited"]);
        assert_eq!(f[0].page, Some(1));
    }

    #[test]
    fn text_layer_carries_the_labels() {
        let t = text_layer(&sample_form()).unwrap();
        assert!(t.contains("Name of person requesting"), "{t}");
    }

    #[test]
    fn fill_flatten_draws_values_and_removes_the_form() {
        let mut values = BTreeMap::new();
        values.insert(
            "RequesterName".to_string(),
            "SYNTHETIC Customer".to_string(),
        );
        values.insert("ReturnMail".to_string(), "yes".to_string());
        let out = fill_flatten(&sample_form(), &values).unwrap();
        assert!(fields(&out).unwrap().is_empty(), "no form layer left");
        let t = text_layer(&out).unwrap();
        assert!(
            t.contains("SYNTHETIC Customer"),
            "value drawn into the page: {t}"
        );
        let mut bad = BTreeMap::new();
        bad.insert("Nope".to_string(), "x".to_string());
        assert!(fill_flatten(&sample_form(), &bad).is_err());
        assert!(fields(b"not a pdf").is_err());
    }
}
