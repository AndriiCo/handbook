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
    seen: &mut std::collections::BTreeSet<ObjectId>,
) {
    if depth > 16 || out.len() > 2_000 {
        return;
    }
    let Object::Reference(id) = obj else { return };
    // Each field object is visited once: shared or cyclic /Kids would
    // otherwise cost exponential work.
    if !seen.insert(*id) {
        return;
    }
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
        walk(doc, f, &name, out, depth + 1, seen);
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
    let mut seen = std::collections::BTreeSet::new();
    for r in &roots {
        walk(&doc, r, "", &mut terminal, 0, &mut seen);
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

/// The PDF's text layer (every page), for label grounding. A page whose
/// text cannot be extracted is skipped, never fatal for the whole form.
pub fn text_layer(bytes: &[u8]) -> Result<String, AcroError> {
    let doc = load(bytes)?;
    let mut out = String::new();
    for n in doc.get_pages().keys().copied().take(200) {
        if let Ok(t) = doc.extract_text(&[n]) {
            out.push_str(&t);
            out.push('\n');
        }
    }
    Ok(out)
}

/// WinAnsi bytes for `s`, or `None` when a character has no WinAnsi
/// glyph (the fill refuses rather than printing `?` on an official form).
fn win_ansi(s: &str) -> Option<Vec<u8>> {
    s.chars()
        .map(|c| match c {
            '\u{20}'..='\u{7e}' => Some(c as u8),
            '\u{a0}'..='\u{ff}' => Some(c as u32 as u8),
            '\u{2018}' | '\u{2019}' => Some(b'\''),
            '\u{201c}' | '\u{201d}' => Some(b'"'),
            '\u{2013}' | '\u{2014}' => Some(b'-'),
            _ => None,
        })
        .collect()
}

fn num(o: &Object) -> Option<f32> {
    o.as_float()
        .ok()
        .or_else(|| o.as_i64().ok().map(|i| i as f32))
}

fn rect_of(d: &Dictionary) -> Option<[f32; 4]> {
    let r = d.get(b"Rect").ok()?.as_array().ok()?;
    let n: Vec<f32> = r.iter().filter_map(num).collect();
    (n.len() == 4).then(|| {
        [
            n[0].min(n[2]),
            n[1].min(n[3]),
            n[0].max(n[2]),
            n[1].max(n[3]),
        ]
    })
}

/// The on-state names of one checkbox/radio widget (`/AP /N` keys
/// other than `Off`).
fn on_states(doc: &Document, widget: &Dictionary) -> Vec<String> {
    widget
        .get(b"AP")
        .ok()
        .and_then(|ap| dict_of(doc, ap))
        .and_then(|ap| ap.get(b"N").ok())
        .and_then(|n| dict_of(doc, n))
        .map(|n| {
            n.iter()
                .map(|(k, _)| String::from_utf8_lossy(k).to_string())
                .filter(|k| k != "Off")
                .collect()
        })
        .unwrap_or_default()
}

/// The normal appearance stream id to draw for a widget: `/AP /N` when
/// it is a stream, else the state named by `state` (or `/AS`).
fn appearance(doc: &Document, widget: &Dictionary, state: Option<&str>) -> Option<ObjectId> {
    let n = widget
        .get(b"AP")
        .ok()
        .and_then(|ap| dict_of(doc, ap))?
        .get(b"N")
        .ok()?;
    let id = n.as_reference().ok();
    match deref(doc, n)? {
        Object::Stream(_) => id,
        Object::Dictionary(states) => {
            let pick = state
                .map(|s| s.as_bytes().to_vec())
                .or_else(|| name_of(widget.get(b"AS").ok()).map(|s| s.into_bytes()))
                .unwrap_or_else(|| b"Off".to_vec());
            states.get(&pick).ok()?.as_reference().ok()
        }
        _ => None,
    }
}

/// Matrix mapping a form XObject's /BBox onto the widget /Rect
/// (identity /Matrix assumed; rotated appearances are rare on forms).
fn bbox_to_rect(doc: &Document, ap: ObjectId, rect: [f32; 4]) -> Option<[f32; 6]> {
    let stream = doc.get_object(ap).ok()?.as_stream().ok()?;
    let b: Vec<f32> = stream
        .dict
        .get(b"BBox")
        .ok()?
        .as_array()
        .ok()?
        .iter()
        .filter_map(num)
        .collect();
    if b.len() != 4 {
        return None;
    }
    let (bw, bh) = ((b[2] - b[0]).abs(), (b[3] - b[1]).abs());
    if bw <= 0.0 || bh <= 0.0 {
        return None;
    }
    let (sx, sy) = ((rect[2] - rect[0]) / bw, (rect[3] - rect[1]) / bh);
    Some([
        sx,
        0.0,
        0.0,
        sy,
        rect[0] - b[0].min(b[2]) * sx,
        rect[1] - b[1].min(b[3]) * sy,
    ])
}

/// Helvetica advance widths are ~0.5 em on average for form text.
const AVG_ADVANCE: f32 = 0.52;

/// Break `text` into lines that fit `width` at `size` (word wrap).
fn wrap(text: &str, size: f32, width: f32) -> Vec<String> {
    let per_line = ((width / (size * AVG_ADVANCE)).floor() as usize).max(1);
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > per_line {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

/// One drawing job for a page: an appearance XObject and/or text.
enum Draw {
    Appearance {
        ap: ObjectId,
        matrix: [f32; 6],
    },
    Text {
        rect: [f32; 4],
        lines: Vec<Vec<u8>>,
        size: f32,
        comb: Option<u32>,
    },
}

/// Draw `values` (keyed by qualified field name) into the pages and
/// remove the form layer. Every visible widget keeps its own appearance
/// (boxes, borders, check marks, seals) burned into the page; a text
/// field with a value gets the value instead of its stored appearance;
/// a checkbox/radio shows the on-state whose export name equals the
/// value (a generic `yes`/`true`/`on`/`1`/`x` only for single-state
/// boxes). Unknown keys, values on read-only or non-fillable fields,
/// and characters outside WinAnsi are errors, never silently dropped.
pub fn fill_flatten(bytes: &[u8], values: &BTreeMap<String, String>) -> Result<Vec<u8>, AcroError> {
    let mut doc = load(bytes)?;
    let known = fields(bytes)?;
    if let Some(bad) = values.keys().find(|k| !known.iter().any(|f| &f.name == *k)) {
        return Err(AcroError(format!("unknown field {bad:?}")));
    }
    for (k, v) in values {
        if win_ansi(v).is_none() {
            return Err(AcroError(format!(
                "field {k:?} has characters this official form cannot print (Latin letters only)"
            )));
        }
    }
    let roots: Vec<Object> = doc
        .catalog()
        .ok()
        .and_then(|c| c.get(b"AcroForm").ok())
        .and_then(|f| dict_of(&doc, f))
        .and_then(|f| f.get(b"Fields").ok())
        .and_then(|f| deref(&doc, f))
        .and_then(|f| f.as_array().ok())
        .cloned()
        .unwrap_or_default();
    let mut terminal = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for r in &roots {
        walk(&doc, r, "", &mut terminal, 0, &mut seen);
    }
    // Widget → page from the pages' own /Annots (authoritative); /P
    // only as a fallback.
    let mut page_of: BTreeMap<ObjectId, ObjectId> = BTreeMap::new();
    for pid in doc.get_pages().values().copied() {
        let annots: Vec<Object> = doc
            .get_dictionary(pid)
            .ok()
            .and_then(|pd| pd.get(b"Annots").ok())
            .and_then(|a| deref(&doc, a))
            .and_then(|a| a.as_array().ok())
            .cloned()
            .unwrap_or_default();
        for a in annots {
            if let Ok(id) = a.as_reference() {
                page_of.entry(id).or_insert(pid);
            }
        }
    }
    let mut per_page: BTreeMap<ObjectId, Vec<Draw>> = BTreeMap::new();
    let mut widget_ids: Vec<ObjectId> = Vec::new();
    for (name, widgets, field_id) in &terminal {
        let Some(field) = known.iter().find(|f| &f.name == name) else {
            continue;
        };
        let fd = doc
            .get_dictionary(*field_id)
            .ok()
            .cloned()
            .unwrap_or_default();
        let ff = inherited(&doc, &fd, b"Ff")
            .and_then(|f| f.as_i64().ok())
            .unwrap_or(0);
        let value = values.get(name).map(|v| v.trim()).filter(|v| !v.is_empty());
        if value.is_some() && (ff & 1 != 0 || matches!(field.kind.as_str(), "button" | "signature"))
        {
            return Err(AcroError(format!(
                "field {name:?} is read-only on this form"
            )));
        }
        let all_states: std::collections::BTreeSet<String> = widgets
            .iter()
            .filter_map(|w| doc.get_dictionary(*w).ok())
            .flat_map(|wd| on_states(&doc, wd))
            .collect();
        for w in widgets {
            widget_ids.push(*w);
            let Ok(wd) = doc.get_dictionary(*w).cloned() else {
                continue;
            };
            let flags = wd.get(b"F").ok().and_then(|f| f.as_i64().ok()).unwrap_or(0);
            if flags & (1 << 1) != 0 || flags & (1 << 5) != 0 || field.kind == "button" {
                continue; // hidden / no-view / push buttons vanish on print
            }
            let Some(rect) = rect_of(&wd) else { continue };
            let Some(page) = page_of
                .get(w)
                .copied()
                .or_else(|| wd.get(b"P").ok().and_then(|p| p.as_reference().ok()))
            else {
                continue;
            };
            let draws = per_page.entry(page).or_default();
            match field.kind.as_str() {
                "checkbox" | "radio" => {
                    let mine = on_states(&doc, &wd);
                    let generic = value.is_some_and(|v| {
                        matches!(v.to_lowercase().as_str(), "yes" | "true" | "on" | "1" | "x")
                    });
                    let chosen = value.and_then(|v| {
                        mine.iter().find(|s| s.as_str() == v).or_else(|| {
                            (generic && all_states.len() == 1)
                                .then(|| mine.first())
                                .flatten()
                        })
                    });
                    let state = chosen.map(String::as_str).or(if value.is_some() {
                        Some("Off")
                    } else {
                        None
                    });
                    if let Some(ap) = appearance(&doc, &wd, state)
                        && let Some(matrix) = bbox_to_rect(&doc, ap, rect)
                    {
                        draws.push(Draw::Appearance { ap, matrix });
                    } else if chosen.is_some() {
                        let size = ((rect[3] - rect[1]) * 0.8).clamp(6.0, 14.0);
                        draws.push(Draw::Text {
                            rect,
                            lines: vec![b"X".to_vec()],
                            size,
                            comb: None,
                        });
                    }
                }
                _ => match value {
                    Some(v) => {
                        let da_size = inherited(&doc, &fd, b"DA")
                            .and_then(|da| da.as_str().ok())
                            .and_then(|da| {
                                let s = String::from_utf8_lossy(da);
                                let toks: Vec<&str> = s.split_whitespace().collect();
                                let i = toks.iter().position(|t| *t == "Tf")?;
                                toks.get(i.checked_sub(1)?)?.parse::<f32>().ok()
                            })
                            .filter(|s| *s > 0.0);
                        let (w_pts, h_pts) = (rect[2] - rect[0] - 4.0, rect[3] - rect[1]);
                        let multiline = ff & (1 << 12) != 0;
                        let comb = (ff & (1 << 24) != 0)
                            .then_some(field.max_len)
                            .flatten()
                            .filter(|n| *n > 0);
                        let mut size = da_size
                            .unwrap_or((h_pts * 0.65).clamp(6.0, 11.0))
                            .min(h_pts.max(4.0));
                        let text = v.to_string();
                        let lines = if multiline {
                            loop {
                                let lines = wrap(&text, size, w_pts);
                                if lines.len() as f32 * size * 1.15 <= h_pts || size <= 5.0 {
                                    break lines;
                                }
                                size -= 0.5;
                            }
                        } else {
                            let fit = w_pts / (text.chars().count().max(1) as f32 * AVG_ADVANCE);
                            if comb.is_none() {
                                size = size.min(fit).max(5.0);
                            }
                            vec![text]
                        };
                        let lines = lines.iter().filter_map(|l| win_ansi(l)).collect();
                        draws.push(Draw::Text {
                            rect,
                            lines,
                            size,
                            comb,
                        });
                    }
                    None => {
                        if let Some(ap) = appearance(&doc, &wd, None)
                            && let Some(matrix) = bbox_to_rect(&doc, ap, rect)
                        {
                            draws.push(Draw::Appearance { ap, matrix });
                        }
                    }
                },
            }
        }
    }
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding",
    });
    // Isolate the original content's graphics state from our drawing.
    let open_q = doc.add_object(Stream::new(Dictionary::new(), b"q\n".to_vec()));
    let close_q = doc.add_object(Stream::new(Dictionary::new(), b"\nQ\n".to_vec()));
    for (page, draws) in per_page {
        let mut ops: Vec<Operation> = Vec::new();
        let mut xobjects: Vec<(String, ObjectId)> = Vec::new();
        for d in draws {
            match d {
                Draw::Appearance { ap, matrix } => {
                    // A widget appearance must be a form XObject to `Do`.
                    if let Ok(Object::Stream(st)) = doc.get_object_mut(ap) {
                        st.dict.set("Type", "XObject");
                        st.dict.set("Subtype", "Form");
                    }
                    let name = format!("HbAp{}", xobjects.len());
                    xobjects.push((name.clone(), ap));
                    ops.extend([
                        Operation::new("q", vec![]),
                        Operation::new("cm", matrix.iter().map(|v| Object::Real(*v)).collect()),
                        Operation::new("Do", vec![Object::Name(name.into_bytes())]),
                        Operation::new("Q", vec![]),
                    ]);
                }
                Draw::Text {
                    rect: [x0, y0, x1, y1],
                    lines,
                    size,
                    comb,
                } => {
                    let h = y1 - y0;
                    ops.extend([
                        Operation::new("q", vec![]),
                        Operation::new(
                            "re",
                            vec![x0.into(), y0.into(), (x1 - x0).into(), h.into()],
                        ),
                        Operation::new("W", vec![]),
                        Operation::new("n", vec![]),
                        Operation::new("BT", vec![]),
                        Operation::new("Tf", vec!["HbFill".into(), size.into()]),
                        Operation::new("rg", vec![0.into(), 0.into(), 0.into()]),
                    ]);
                    if let (Some(cells), Some(line)) = (comb, lines.first()) {
                        let cell = (x1 - x0) / cells as f32;
                        for (i, b) in line.iter().take(cells as usize).enumerate() {
                            let x = x0 + cell * i as f32 + (cell - size * AVG_ADVANCE) / 2.0;
                            ops.push(Operation::new(
                                "Tm",
                                vec![
                                    1.into(),
                                    0.into(),
                                    0.into(),
                                    1.into(),
                                    x.into(),
                                    (y0 + (h - size) / 2.0 + size * 0.2).into(),
                                ],
                            ));
                            ops.push(Operation::new(
                                "Tj",
                                vec![Object::String(vec![*b], StringFormat::Literal)],
                            ));
                        }
                    } else if lines.len() > 1 {
                        let lead = size * 1.15;
                        for (i, line) in lines.into_iter().enumerate() {
                            let y = y1 - 2.0 - size - lead * i as f32;
                            ops.push(Operation::new(
                                "Tm",
                                vec![
                                    1.into(),
                                    0.into(),
                                    0.into(),
                                    1.into(),
                                    (x0 + 2.0).into(),
                                    y.into(),
                                ],
                            ));
                            ops.push(Operation::new(
                                "Tj",
                                vec![Object::String(line, StringFormat::Literal)],
                            ));
                        }
                    } else if let Some(line) = lines.into_iter().next() {
                        ops.push(Operation::new(
                            "Tm",
                            vec![
                                1.into(),
                                0.into(),
                                0.into(),
                                1.into(),
                                (x0 + 2.0).into(),
                                (y0 + (h - size) / 2.0 + size * 0.2).into(),
                            ],
                        ));
                        ops.push(Operation::new(
                            "Tj",
                            vec![Object::String(line, StringFormat::Literal)],
                        ));
                    }
                    ops.extend([Operation::new("ET", vec![]), Operation::new("Q", vec![])]);
                }
            }
        }
        let content = Content { operations: ops }
            .encode()
            .map_err(err("encode content"))?;
        let stream_id = doc.add_object(Stream::new(Dictionary::new(), content));
        let page_dict = doc.get_dictionary(page).map_err(err("page"))?.clone();
        let original: Vec<Object> = match page_dict.get(b"Contents") {
            Ok(Object::Array(a)) => a.clone(),
            Ok(Object::Reference(r)) => match doc.get_object(*r) {
                Ok(Object::Array(a)) => a.clone(),
                _ => vec![Object::Reference(*r)],
            },
            _ => Vec::new(),
        };
        let mut contents = vec![Object::Reference(open_q)];
        contents.extend(original);
        contents.push(Object::Reference(close_q));
        contents.push(Object::Reference(stream_id));
        // Effective resources, including those inherited from /Pages.
        let mut resources = inherited(&doc, &page_dict, b"Resources")
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
        if !xobjects.is_empty() {
            let mut xo = resources
                .get(b"XObject")
                .ok()
                .and_then(|f| dict_of(&doc, f))
                .cloned()
                .unwrap_or_default();
            for (name, id) in xobjects {
                xo.set(name.into_bytes(), Object::Reference(id));
            }
            resources.set("XObject", Object::Dictionary(xo));
        }
        let page_mut = doc.get_dictionary_mut(page).map_err(err("page"))?;
        page_mut.set("Contents", Object::Array(contents));
        page_mut.set("Resources", Object::Dictionary(resources));
    }
    // Flatten: drop the form widgets (links and other annotations stay),
    // the form, and usage-rights signatures the edit invalidates.
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
        cat.remove(b"Perms");
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
        sample_form_with(false)
    }

    /// `inherit`: fonts live on the /Pages node, not the page.
    pub fn sample_form_with(inherit: bool) -> Vec<u8> {
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
        let mail_ap = doc.add_object(Stream::new(
            dictionary! {"BBox" => vec![0.into(), 0.into(), 12.into(), 12.into()]},
            b"0 0 m 12 12 l S".to_vec(),
        ));
        let pick_ap = doc.add_object(Stream::new(
            dictionary! {"BBox" => vec![0.into(), 0.into(), 12.into(), 12.into()]},
            b"12 0 m 0 12 l S".to_vec(),
        ));
        let off_ap = doc.add_object(Stream::new(
            dictionary! {"BBox" => vec![0.into(), 0.into(), 12.into(), 12.into()]},
            b"0 0 12 12 re S".to_vec(),
        ));
        let radio = doc.new_object_id();
        let w_mail = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Parent" => Object::Reference(radio),
            "Rect" => vec![250.into(), 600.into(), 262.into(), 612.into()], "P" => Object::Reference(page_id),
            "AP" => dictionary! {"N" => dictionary! {"Mail" => Object::Reference(mail_ap), "Off" => Object::Reference(off_ap)}},
        });
        let w_pick = doc.add_object(dictionary! {
            "Type" => "Annot", "Subtype" => "Widget", "Parent" => Object::Reference(radio),
            "Rect" => vec![300.into(), 600.into(), 312.into(), 612.into()], "P" => Object::Reference(page_id),
            "AP" => dictionary! {"N" => dictionary! {"Pickup" => Object::Reference(pick_ap), "Off" => Object::Reference(off_ap)}},
        });
        doc.objects.insert(
            radio,
            Object::Dictionary(dictionary! {
                "FT" => "Btn", "Ff" => (1i64 << 15), "T" => Object::string_literal("Delivery"),
                "Kids" => vec![Object::Reference(w_mail), Object::Reference(w_pick)],
            }),
        );
        let link = doc.add_object(dictionary! {"Type" => "Annot", "Subtype" => "Link", "Rect" => vec![0.into(), 0.into(), 10.into(), 10.into()]});
        let ro = doc.add_object(dictionary! {
            "FT" => "Tx", "T" => Object::string_literal("OfficeUse"), "Ff" => 1, "Type" => "Annot", "Subtype" => "Widget",
            "Rect" => vec![250.into(), 560.into(), 400.into(), 575.into()], "P" => Object::Reference(page_id),
        });
        let fonts = dictionary! {"Font" => dictionary! {"F1" => Object::Reference(font)}};
        let mut page = dictionary! {
            "Type" => "Page", "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => Object::Reference(content_id),
            "Annots" => vec![Object::Reference(text), Object::Reference(check), Object::Reference(choice),
                             Object::Reference(w_mail), Object::Reference(w_pick), Object::Reference(link), Object::Reference(ro)],
        };
        let mut pages = dictionary! {"Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1};
        if inherit {
            pages.set("Resources", Object::Dictionary(fonts));
        } else {
            page.set("Resources", Object::Dictionary(fonts));
        }
        doc.objects.insert(page_id, Object::Dictionary(page));
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let form = doc.add_object(dictionary! {
            "Fields" => vec![Object::Reference(text), Object::Reference(check), Object::Reference(choice),
                             Object::Reference(radio), Object::Reference(ro)],
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
                ("Speed", "choice"),
                ("Delivery", "radio"),
                ("OfficeUse", "text")
            ]
        );
        assert_eq!(f[3].options, ["Mail", "Pickup"]);
        assert_eq!(f[1].options, ["Yes"]);
        assert_eq!(f[2].options, ["Standard", "Expedited"]);
        assert_eq!(f[0].page, Some(1));
    }

    #[test]
    fn text_layer_carries_the_labels() {
        let t = text_layer(&sample_form()).unwrap();
        assert!(t.contains("Name of person requesting"), "{t}");
    }

    /// Appearance streams the flattened page draws (its /XObject map).
    fn drawn_aps(pdf: &[u8]) -> Vec<Vec<u8>> {
        let doc = Document::load_mem(pdf).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        let pd = doc.get_dictionary(page).unwrap();
        let res = inherited(&doc, pd, b"Resources")
            .and_then(|r| dict_of(&doc, r))
            .unwrap();
        res.get(b"XObject")
            .ok()
            .and_then(|x| dict_of(&doc, x))
            .map(|x| {
                x.iter()
                    .filter_map(|(_, v)| v.as_reference().ok())
                    .filter_map(|id| {
                        doc.get_object(id)
                            .ok()?
                            .as_stream()
                            .ok()
                            .map(|s| s.content.clone())
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    #[test]
    fn radio_draws_only_the_chosen_option() {
        let src = sample_form();
        let mut values = BTreeMap::new();
        values.insert("Delivery".to_string(), "Pickup".to_string());
        let bodies = drawn_aps(&fill_flatten(&src, &values).unwrap());
        assert!(
            bodies.contains(&b"12 0 m 0 12 l S".to_vec()),
            "Pickup on-state drawn"
        );
        assert!(
            !bodies.contains(&b"0 0 m 12 12 l S".to_vec()),
            "Mail never ticked"
        );
        assert!(
            bodies.contains(&b"0 0 12 12 re S".to_vec()),
            "the other box keeps its Off square"
        );
        let mut generic = BTreeMap::new();
        generic.insert("Delivery".to_string(), "yes".to_string());
        let bodies = drawn_aps(&fill_flatten(&src, &generic).unwrap());
        assert!(
            bodies.iter().all(|b| b == b"0 0 12 12 re S"),
            "a generic yes never ticks a multi-option radio"
        );
    }

    #[test]
    fn inherited_resources_survive_and_links_stay() {
        let src = sample_form_with(true);
        let mut values = BTreeMap::new();
        values.insert(
            "RequesterName".to_string(),
            "SYNTHETIC Customer".to_string(),
        );
        let out = fill_flatten(&src, &values).unwrap();
        let doc = Document::load_mem(&out).unwrap();
        let page = *doc.get_pages().values().next().unwrap();
        let pd = doc.get_dictionary(page).unwrap();
        let res = inherited(&doc, pd, b"Resources")
            .and_then(|r| dict_of(&doc, r))
            .unwrap();
        let fonts = res
            .get(b"Font")
            .ok()
            .and_then(|f| dict_of(&doc, f))
            .unwrap();
        assert!(fonts.has(b"F1"), "the page's own labels keep their font");
        assert!(fonts.has(b"HbFill"));
        let annots = pd.get(b"Annots").unwrap().as_array().unwrap();
        assert_eq!(annots.len(), 1, "only the link annotation remains");
        assert!(
            text_layer(&out)
                .unwrap()
                .contains("Name of person requesting")
        );
    }

    #[test]
    fn refuses_read_only_fields_and_unprintable_text() {
        let src = sample_form();
        let mut ro = BTreeMap::new();
        ro.insert("OfficeUse".to_string(), "x".to_string());
        assert!(fill_flatten(&src, &ro).unwrap_err().0.contains("read-only"));
        let mut vn = BTreeMap::new();
        vn.insert("RequesterName".to_string(), "Nguyễn Văn A".to_string());
        assert!(
            fill_flatten(&src, &vn)
                .unwrap_err()
                .0
                .contains("cannot print")
        );
        let mut latin = BTreeMap::new();
        latin.insert("RequesterName".to_string(), "José Müller".to_string());
        assert!(fill_flatten(&src, &latin).is_ok(), "Latin-1 names print");
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
