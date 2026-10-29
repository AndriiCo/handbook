//! Status-table rendering: result JSON → HTML + SVG.
//!
//! When a task result carries `{columns, rows[, status]}`, the
//! runner renders an embeddable HTML table and an SVG grid. Cell
//! geometry is computed from content; world-map drawing is pack
//! content (a geometry file + a custom renderer), not harness code.

use serde_json::Value;

/// One rendered row: cells parallel to `columns`, optional status.
#[derive(Debug, Clone)]
pub struct StatusRow {
    pub cells: Vec<String>,
    pub status: String,
}

/// Extract `{columns, rows[, status]}` from a task result.
/// Returns `None` when the shape is absent (not an error — most
/// tasks produce no table).
pub fn table_from_result(result: &Value) -> Option<(Vec<String>, Vec<StatusRow>)> {
    let columns = result
        .get("columns")?
        .as_array()?
        .iter()
        .map(|c| c.as_str().map(str::to_string))
        .collect::<Option<Vec<_>>>()?;
    let rows = result.get("rows")?.as_array()?;
    let statuses = result
        .get("status")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::with_capacity(rows.len());
    for (i, row) in rows.iter().enumerate() {
        let cells = row.as_array()?.iter().map(cell_text).collect::<Vec<_>>();
        let status = statuses
            .get(i)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        out.push(StatusRow { cells, status });
    }
    Some((columns, out))
}

fn cell_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render an embeddable HTML table.
pub fn render_status_html(columns: &[String], rows: &[StatusRow]) -> String {
    let mut out = String::from("<table class=\"atlas-status\">\n<thead><tr>");
    for column in columns {
        out.push_str(&format!("<th>{}</th>", escape(column)));
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for row in rows {
        let class = match row.status.as_str() {
            "ok" => " class=\"ok\"",
            "warn" => " class=\"warn\"",
            "bad" => " class=\"bad\"",
            _ => "",
        };
        out.push_str(&format!("<tr{class}>"));
        for cell in &row.cells {
            out.push_str(&format!("<td>{}</td>", escape(cell)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n");
    out
}

/// Render an SVG grid (monospace, status-tinted row markers).
pub fn render_status_svg(columns: &[String], rows: &[StatusRow]) -> String {
    const CHAR_W: usize = 8;
    const PAD: usize = 10;
    const ROW_H: usize = 22;
    const MARK_W: usize = 6;
    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, column)| {
            let longest = rows
                .iter()
                .filter_map(|row| row.cells.get(i))
                .map(|cell| cell.chars().count())
                .max()
                .unwrap_or(0)
                .max(column.chars().count());
            longest * CHAR_W + PAD * 2
        })
        .collect();
    let width: usize = MARK_W + widths.iter().sum::<usize>();
    let height = ROW_H * (rows.len() + 1) + 2;
    let mut out = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         font-family=\"monospace\" font-size=\"12\">\n"
    );
    let mut x = MARK_W;
    for (i, column) in columns.iter().enumerate() {
        out.push_str(&format!(
            "<rect x=\"{x}\" y=\"1\" width=\"{}\" height=\"{ROW_H}\" fill=\"#eee\"/>\
             <text x=\"{}\" y=\"16\">{}</text>\n",
            widths[i],
            x + PAD,
            escape(column)
        ));
        x += widths[i];
    }
    for (r, row) in rows.iter().enumerate() {
        let y = 1 + ROW_H * (r + 1);
        let fill = match row.status.as_str() {
            "ok" => "#4c4",
            "warn" => "#eb3",
            "bad" => "#e44",
            _ => "#ccc",
        };
        out.push_str(&format!(
            "<rect x=\"0\" y=\"{y}\" width=\"{MARK_W}\" height=\"{ROW_H}\" fill=\"{fill}\"/>\n"
        ));
        let mut x = MARK_W;
        for (i, cell) in row.cells.iter().enumerate() {
            let w = widths.get(i).copied().unwrap_or(60);
            out.push_str(&format!(
                "<rect x=\"{x}\" y=\"{y}\" width=\"{w}\" height=\"{ROW_H}\" fill=\"none\" stroke=\"#999\"/>\
                 <text x=\"{}\" y=\"{}\">{}</text>\n",
                x + PAD,
                y + 15,
                escape(cell)
            ));
            x += w;
        }
    }
    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "columns": ["State", "EIF"],
            "rows": [["Viet Nam", "2026-09-11"], ["Egypt", "—"]],
            "status": ["ok", "bad"],
        })
    }

    #[test]
    fn table_shape_extracts() {
        let (columns, rows) = table_from_result(&sample()).unwrap();
        assert_eq!(columns, vec!["State", "EIF"]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].status, "ok");
        assert!(table_from_result(&json!({"rows": []})).is_none());
    }

    #[test]
    fn html_escapes_and_marks_status() {
        let (columns, rows) = table_from_result(&sample()).unwrap();
        let html = render_status_html(&columns, &rows);
        assert!(html.contains("<th>State</th>"));
        assert!(html.contains("<tr class=\"ok\">"));
        assert!(html.contains("<td>Viet Nam</td>"));
        let evil = render_status_html(
            &["<x>".to_string()],
            &[StatusRow {
                cells: vec!["<b>&".to_string()],
                status: String::new(),
            }],
        );
        assert!(evil.contains("&lt;b&gt;&amp;"));
        assert!(!evil.contains("<b>"));
    }

    #[test]
    fn svg_grid_has_markers_and_cells() {
        let (columns, rows) = table_from_result(&sample()).unwrap();
        let svg = render_status_svg(&columns, &rows);
        assert!(svg.contains("<svg"));
        assert!(svg.contains("Viet Nam"));
        assert!(svg.contains("fill=\"#4c4\""));
        assert!(svg.contains("fill=\"#e44\""));
    }
}
