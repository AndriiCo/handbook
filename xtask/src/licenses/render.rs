//! The page: one self-contained HTML file (inline CSS and a small
//! progressive-enhancement script, no external requests).

use std::collections::BTreeMap;

use super::texts::{self, Library, Tok};
use super::{Bom, Row, Scope};

const REPO: &str = "https://github.com/AndriiCo/handbook";

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn scope_blurb(scope: Scope) -> &'static str {
    match scope {
        Scope::App => {
            "Runs in production: the andrii.ai web server, or ships inside the web app's JavaScript."
        }
        Scope::Worker => "Part of xtask: the plant worker, scheduled jobs and operator tools.",
        Scope::Build => {
            "Builds and ships a release, or runs inside the compiler (build scripts, procedural macros)."
        }
        Scope::Test => "Used only by the test suites, CI and the rehearsal server.",
        Scope::Image => "Inside the Docker image judges run with docker compose.",
    }
}

/// `https://www.github.com/a/b/` → `github.com/a/b`.
fn short_url(url: &str) -> String {
    let s = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_end_matches('/');
    s.to_string()
}

fn licence_cell(row: &Row) -> String {
    let mut out = String::new();
    if row.expr.is_empty() {
        out.push_str("<span class=\"muted\">Not declared</span>");
    }
    let mut prev_open = true;
    for tok in texts::tokens(&row.expr) {
        let piece = match &tok {
            Tok::Id(id) | Tok::Exception(id) => {
                let anchor = row.links.get(id).cloned().unwrap_or_default();
                format!("<a href=\"#{}\">{}</a>", esc(&anchor), esc(id))
            }
            Tok::Op(op) => format!("<span class=\"op\">{op}</span>"),
            Tok::Open => "(".to_string(),
            Tok::Close => ")".to_string(),
        };
        if !prev_open && tok != Tok::Close {
            out.push(' ');
        }
        prev_open = tok == Tok::Open;
        out.push_str(&piece);
    }
    if row.detected {
        out.push_str(
            " <span class=\"flag\" title=\"The package declares no licence; this is read from its licence file\">detected</span>",
        );
    }
    for anchor in &row.own_texts {
        out.push_str(&format!(
            " <a class=\"flag\" href=\"#{}\">additional terms</a>",
            esc(anchor)
        ));
    }
    for anchor in &row.notices {
        out.push_str(&format!(
            " <a class=\"flag\" href=\"#{}\">NOTICE</a>",
            esc(anchor)
        ));
    }
    out
}

fn copyright_cell(row: &Row) -> String {
    let lines = &row.pkg.copyrights;
    if lines.is_empty() {
        return if row.pkg.authors.is_empty() {
            "<span class=\"muted\">—</span>".to_string()
        } else {
            format!(
                "<span class=\"muted\">Authors: {}</span>",
                esc(&row.pkg.authors.join(", "))
            )
        };
    }
    let shown = 3;
    let mut out: Vec<String> = lines.iter().take(shown).map(|l| esc(l)).collect();
    if lines.len() > shown {
        let rest: Vec<String> = lines[shown..].iter().map(|l| esc(l)).collect();
        out.push(format!(
            "<details><summary>{} more</summary>{}</details>",
            rest.len(),
            rest.join("<br>")
        ));
    }
    out.join("<br>")
}

fn table_row(row: &Row) -> String {
    let p = &row.pkg;
    let scopes: Vec<&str> = p.scopes.iter().map(|s| s.as_str()).collect();
    let chips: String = p
        .scopes
        .iter()
        .map(|s| format!("<span class=\"chip s-{0}\">{0}</span>", s.as_str()))
        .collect();
    let note = p
        .note
        .as_deref()
        .map(|n| format!("<span class=\"note\">{}</span>", esc(n)))
        .unwrap_or_default();
    format!(
        "<tr id=\"{anchor}\" data-scope=\"{scope}\" data-eco=\"{eco}\">\
         <td class=\"pkg\"><span class=\"name\">{name}</span>{note}</td>\
         <td class=\"ver\">{version}</td><td class=\"eco\">{eco}</td><td class=\"scope\">{chips}</td>\
         <td class=\"lic\">{licence}</td><td class=\"cr\">{copyright}</td>\
         <td class=\"src\"><a href=\"{url}\" rel=\"noopener\">{short}</a></td></tr>\n",
        anchor = esc(&row.anchor),
        scope = scopes.join(" "),
        eco = esc(&p.ecosystem),
        name = esc(&p.name),
        version = esc(&p.version),
        licence = licence_cell(row),
        copyright = copyright_cell(row),
        url = esc(&p.url),
        short = esc(&short_url(&p.url)),
    )
}

fn users(bom: &Bom, idx: &[usize]) -> String {
    let links: Vec<String> = idx
        .iter()
        .map(|i| {
            let row = &bom.rows[*i];
            format!(
                "<a href=\"#{}\">{} {}</a>",
                esc(&row.anchor),
                esc(&row.pkg.name),
                esc(&row.pkg.version)
            )
        })
        .collect();
    let noun = if idx.len() == 1 {
        "package"
    } else {
        "packages"
    };
    format!(
        "<details class=\"users\"><summary>Used by {} {noun}</summary><p>{}</p></details>",
        idx.len(),
        links.join(", ")
    )
}

/// Licence ids by how many rows link to them (most used first).
fn licence_counts(bom: &Bom) -> Vec<(String, usize)> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for row in &bom.rows {
        for id in row.links.keys() {
            *counts.entry(id.clone()).or_default() += 1;
        }
    }
    for variant in bom.variants.values() {
        counts.entry(variant.primary.clone()).or_default();
    }
    let mut list: Vec<(String, usize)> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    list
}

fn licence_sections(bom: &Bom, lib: &Library) -> String {
    let mut out = String::new();
    for (id, _) in licence_counts(bom) {
        if let Some(users_idx) = bom.canonical.get(&id) {
            let source = lib.source(&id);
            out.push_str(&format!(
                "<section class=\"text\" id=\"license-{anchor}\">\
                 <h3>{name}</h3><p class=\"meta\"><code>{id}</code> · {source} · \
                 <a href=\"#bom\">back to the table</a></p>{users}<pre>{body}</pre></section>\n",
                anchor = esc(&super::slug(&id)),
                name = esc(lib.name(&id)),
                id = esc(&id),
                source = esc(&source),
                users = users(bom, users_idx),
                body = esc(lib.texts.get(&id).map_or("", |t| t.body.trim_end())),
            ));
        }
        for (anchor, variant) in bom.variants.iter().filter(|(_, v)| v.primary == id) {
            let first = &bom.rows[variant.users[0]].pkg;
            let base = if lib.texts.contains_key(&id) {
                format!("{} ({})", lib.name(&id), id)
            } else {
                "any listed licence".to_string()
            };
            let (title, meta) = if variant.close {
                (
                    format!("{}: as shipped by {}", lib.name(&id), first.name),
                    format!(
                        "Differs from the canonical {base} text beyond its copyright lines, so it \
                         is reproduced as shipped. Copyright notices are in each package's row."
                    ),
                )
            } else {
                (
                    format!("Additional licence file shipped by {}", first.name),
                    format!(
                        "Covers parts of the package beyond its main licence; the nearest \
                         standard text is {base}. Reproduced as shipped, without copyright lines."
                    ),
                )
            };
            out.push_str(&format!(
                "<section class=\"text variant\" id=\"{anchor}\">\
                 <h3>{title}</h3><p class=\"meta\">{meta} · \
                 <a href=\"#bom\">back to the table</a></p>{users}<pre>{body}</pre></section>\n",
                anchor = esc(anchor),
                title = esc(&title),
                meta = esc(&meta),
                users = users(bom, &variant.users),
                body = esc(&variant.text),
            ));
        }
    }
    out
}

fn notice_sections(bom: &Bom) -> String {
    let mut out = String::new();
    for (anchor, notice) in &bom.notices {
        let names: Vec<String> = notice
            .users
            .iter()
            .map(|i| bom.rows[*i].pkg.name.clone())
            .collect();
        out.push_str(&format!(
            "<section class=\"text\" id=\"{anchor}\"><h3>NOTICE: {names}</h3>\
             <p class=\"meta\"><a href=\"#bom\">back to the table</a></p>{users}\
             <pre>{body}</pre></section>\n",
            anchor = esc(anchor),
            names = esc(&names.join(", ")),
            users = users(bom, &notice.users),
            body = esc(&notice.text),
        ));
    }
    out
}

pub(crate) fn page(bom: &Bom, lib: &Library, generated: &str, inputs: &str) -> String {
    let total = bom.rows.len();
    let count_eco = |eco: &str| bom.rows.iter().filter(|r| r.pkg.ecosystem == eco).count();
    let crates = count_eco("crates.io");
    let npm = count_eco("npm");
    let licences = licence_counts(bom);
    let glance: String = licences
        .iter()
        .filter(|(id, _)| bom.canonical.contains_key(id))
        .map(|(id, n)| {
            format!(
                "<a class=\"lchip\" href=\"#license-{}\">{}<span>{n}</span></a>",
                esc(&super::slug(id)),
                esc(id)
            )
        })
        .collect();
    let legend: String = Scope::ALL
        .iter()
        .map(|s| {
            format!(
                "<div><dt><span class=\"chip s-{0}\">{0}</span></dt><dd>{1}</dd></div>",
                s.as_str(),
                esc(scope_blurb(*s))
            )
        })
        .collect();
    let filters: String = std::iter::once(("", "All"))
        .chain(Scope::ALL.iter().map(|s| (s.as_str(), s.as_str())))
        .map(|(value, label)| {
            format!(
                "<button type=\"button\" data-filter-scope=\"{value}\" aria-pressed=\"{}\">{label}</button>",
                value.is_empty()
            )
        })
        .collect();
    let mut ecosystems: Vec<&str> = bom.rows.iter().map(|r| r.pkg.ecosystem.as_str()).collect();
    ecosystems.dedup();
    let eco_options: String = ecosystems
        .iter()
        .map(|e| format!("<option value=\"{0}\">{0}</option>", esc(e)))
        .collect();
    let rows: String = bom.rows.iter().map(table_row).collect();
    let notices = if bom.notices.is_empty() {
        String::new()
    } else {
        format!(
            "<h2 id=\"notices\">Notices</h2>\n<p>Apache-2.0 and some other licences require a \
             package's NOTICE file to travel with it. These are reproduced as shipped.</p>\n{}",
            notice_sections(bom)
        )
    };

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<!-- Generated by `cargo xtask licenses` (docs/licenses/README.md). Do not edit by hand. -->
<meta name="bom:generated" content="{generated}">
<meta name="bom:inputs" content="{inputs}">
<title>Open-source licences · Apostille Handbook</title>
<meta name="description" content="Every open-source component in the Apostille Handbook at andrii.ai, with versions, sources, copyright notices and licence texts.">
<link rel="icon" href="/apostille/favicon.svg" type="image/svg+xml">
<style>
:root{{color-scheme:light dark;--bg:#f7f6f3;--surface:#fff;--text:#1c1e22;--muted:#5d636c;--line:#e2dfd8;--soft:#f0eee9;--accent:#2a5b8c;--hit:#fff3c4;
--app-bg:#e3eefb;--app-fg:#1f4f86;--worker-bg:#dff3ee;--worker-fg:#17675a;--build-bg:#fbefd9;--build-fg:#8a5310;--test-bg:#eee7fb;--test-fg:#5b3d99;--image-bg:#e9ecf0;--image-fg:#3f4a57}}
@media (prefers-color-scheme:dark){{:root:not([data-theme="light"]){{--bg:#131518;--surface:#1b1e22;--text:#e7e8ea;--muted:#9aa1ab;--line:#2d3138;--soft:#22262b;--accent:#8bb8e8;--hit:#4a3f12;
--app-bg:#1d3047;--app-fg:#a9cdf5;--worker-bg:#173a35;--worker-fg:#9fe0d3;--build-bg:#3d2e15;--build-fg:#f2c98a;--test-bg:#2f2547;--test-fg:#cdb8f5;--image-bg:#2a3038;--image-fg:#c3cbd5}}}}
:root[data-theme="dark"]{{--bg:#131518;--surface:#1b1e22;--text:#e7e8ea;--muted:#9aa1ab;--line:#2d3138;--soft:#22262b;--accent:#8bb8e8;--hit:#4a3f12;
--app-bg:#1d3047;--app-fg:#a9cdf5;--worker-bg:#173a35;--worker-fg:#9fe0d3;--build-bg:#3d2e15;--build-fg:#f2c98a;--test-bg:#2f2547;--test-fg:#cdb8f5;--image-bg:#2a3038;--image-fg:#c3cbd5}}
*{{box-sizing:border-box}}
body{{margin:0;background:var(--bg);color:var(--text);font:15px/1.55 system-ui,-apple-system,"Segoe UI",sans-serif}}
a{{color:var(--accent);text-decoration-thickness:1px;text-underline-offset:2px}}
code,pre,.ver{{font-family:ui-monospace,SFMono-Regular,Menlo,Consolas,monospace}}
.top{{border-bottom:1px solid var(--line);background:var(--surface)}}
.top div{{max-width:80rem;margin:0 auto;padding:.7rem 16px;display:flex;gap:.6rem;align-items:baseline}}
.top a{{color:var(--text);font-weight:600;text-decoration:none}}.top span{{color:var(--muted)}}
main{{max-width:80rem;margin:0 auto;padding:1.5rem 16px 3rem}}
h1{{font:600 clamp(1.6rem,3.5vw,2.2rem)/1.2 ui-serif,Georgia,serif;margin:.4rem 0 .8rem}}
h2{{font:600 1.35rem/1.3 ui-serif,Georgia,serif;margin:2.8rem 0 .6rem;padding-top:.4rem}}
h3{{font-size:1.05rem;margin:0 0 .2rem}}
.lede{{max-width:46rem;color:var(--muted);font-size:1.02rem}}
.stats{{display:grid;grid-template-columns:repeat(auto-fit,minmax(9rem,1fr));gap:.6rem;margin:1.4rem 0}}
.stat{{background:var(--surface);border:1px solid var(--line);border-radius:10px;padding:.7rem .9rem}}
.stat b{{display:block;font-size:1.5rem;font-variant-numeric:tabular-nums}}.stat span{{color:var(--muted);font-size:.85rem}}
dl.legend{{display:grid;grid-template-columns:repeat(auto-fit,minmax(17rem,1fr));gap:.4rem 1.2rem;margin:1rem 0}}
dl.legend div{{display:flex;gap:.6rem;align-items:baseline}}dl.legend dt{{flex:none;width:4.2rem}}dl.legend dd{{margin:0;color:var(--muted);font-size:.9rem}}
.glance{{display:flex;flex-wrap:wrap;gap:.35rem;margin:.6rem 0 0}}
.lchip{{display:inline-flex;gap:.4rem;align-items:center;border:1px solid var(--line);background:var(--surface);border-radius:99px;padding:.15rem .6rem;font-size:.82rem;text-decoration:none;color:var(--text)}}
.lchip span{{color:var(--muted);font-variant-numeric:tabular-nums}}.lchip:hover{{border-color:var(--accent)}}
.chip{{display:inline-block;border-radius:5px;padding:0 .4rem;margin:0 .2rem .15rem 0;font-size:.75rem;font-weight:600;line-height:1.5}}
.s-app{{background:var(--app-bg);color:var(--app-fg)}}.s-worker{{background:var(--worker-bg);color:var(--worker-fg)}}.s-build{{background:var(--build-bg);color:var(--build-fg)}}.s-test{{background:var(--test-bg);color:var(--test-fg)}}.s-image{{background:var(--image-bg);color:var(--image-fg)}}
.controls{{display:flex;flex-wrap:wrap;gap:.5rem;align-items:center;margin:1.6rem 0 .6rem}}
.controls input,.controls select{{font:inherit;color:var(--text);background:var(--surface);border:1px solid var(--line);border-radius:8px;padding:.4rem .6rem}}
.controls input{{flex:1 1 14rem;min-width:0}}
.seg{{display:flex;flex-wrap:wrap;gap:.25rem}}
.seg button{{font:inherit;font-size:.85rem;color:var(--text);background:var(--surface);border:1px solid var(--line);border-radius:8px;padding:.3rem .65rem;cursor:pointer}}
.seg button[aria-pressed="true"]{{background:var(--text);color:var(--bg);border-color:var(--text)}}
#count{{color:var(--muted);font-size:.85rem;margin-left:auto;font-variant-numeric:tabular-nums}}
.wrap{{overflow:auto;max-height:78vh;border:1px solid var(--line);border-radius:10px;background:var(--surface)}}
table{{border-collapse:separate;border-spacing:0;width:100%;font-size:.84rem}}
th{{position:sticky;top:0;z-index:1;background:var(--soft);text-align:left;font-weight:600;padding:.55rem .7rem;border-bottom:1px solid var(--line);white-space:nowrap}}
td{{padding:.5rem .7rem;border-bottom:1px solid var(--line);vertical-align:top}}
tbody tr:last-child td{{border-bottom:0}}tbody tr:target td,tbody tr.hit td{{background:var(--hit)}}
.pkg .name{{font-weight:600;word-break:break-word}}.note{{display:block;color:var(--muted);font-size:.78rem;margin-top:.15rem;max-width:22rem}}
.ver,.scope{{white-space:nowrap}}.ver{{font-size:.8rem}}.eco{{white-space:nowrap;color:var(--muted)}}
.lic{{min-width:9rem}}.op{{color:var(--muted);font-size:.72rem;letter-spacing:.03em}}
.flag{{font-size:.7rem;border:1px solid var(--line);border-radius:4px;padding:0 .3rem;color:var(--muted);white-space:nowrap}}
.cr{{color:var(--muted);font-size:.78rem;min-width:14rem;max-width:26rem;overflow-wrap:anywhere}}
.cr summary{{cursor:pointer;color:var(--accent)}}
.src a{{display:inline-block;max-width:15rem;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;vertical-align:bottom}}
.muted{{color:var(--muted)}}#empty{{margin:0;padding:1.2rem;color:var(--muted);text-align:center}}
section.text{{overflow-wrap:anywhere;background:var(--surface);border:1px solid var(--line);border-radius:10px;padding:1rem 1.1rem;margin:0 0 .9rem;scroll-margin-top:1rem}}
section.text:target{{border-color:var(--accent);box-shadow:0 0 0 2px var(--accent) inset}}
section.variant{{border-style:dashed}}
.meta{{color:var(--muted);font-size:.84rem;margin:0 0 .5rem}}
.users summary{{cursor:pointer;font-size:.86rem;color:var(--accent)}}.users p{{font-size:.82rem;margin:.4rem 0 .6rem;line-height:1.7}}
pre{{white-space:pre-wrap;overflow-wrap:anywhere;font-size:.78rem;line-height:1.5;background:var(--soft);border-radius:8px;padding:.8rem .9rem;margin:.4rem 0 0;max-height:28rem;overflow:auto}}
footer{{border-top:1px solid var(--line);margin-top:3rem;padding-top:1rem;color:var(--muted);font-size:.86rem}}
@media (max-width:640px){{body{{font-size:14px}}#count{{margin-left:0;width:100%}}.wrap{{max-height:none}}th{{position:static}}}}
</style>
</head>
<body>
<header class="top"><div><a href="/apostille/">Apostille Handbook</a><span>/ Licences</span></div></header>
<main>
<h1>Open-source software and licences</h1>
<p class="lede">The Apostille Handbook is built on open-source software. This page lists every third-party component in the web app, the plant workers, and the build and release tooling, with the version in use, where its source lives, its copyright notices and its licence. Each licence text appears once below the table; copies that differ only by their copyright lines share it. The Handbook's own code is covered by <a href="{REPO}/blob/main/LICENSE.md">its licence</a>.</p>
<div class="stats">
<div class="stat"><b>{total}</b><span>components</span></div>
<div class="stat"><b>{crates}</b><span>Rust crates</span></div>
<div class="stat"><b>{npm}</b><span>npm packages</span></div>
<div class="stat"><b>{other}</b><span>toolchains, images and tools</span></div>
<div class="stat"><b>{licence_count}</b><span>distinct licences</span></div>
</div>
<dl class="legend">{legend}</dl>
<div class="glance">{glance}</div>
<div class="controls" id="controls" hidden>
<input id="q" type="search" placeholder="Filter by name, licence or copyright" aria-label="Filter packages">
<div class="seg" role="group" aria-label="Scope">{filters}</div>
<select id="eco" aria-label="Ecosystem"><option value="">All ecosystems</option>{eco_options}</select>
<span id="count" aria-live="polite"></span>
</div>
<div class="wrap" id="bom">
<table>
<thead><tr><th>Package</th><th>Version</th><th>Ecosystem</th><th>Scope</th><th>Licence</th><th>Copyright</th><th>Source</th></tr></thead>
<tbody>
{rows}</tbody>
</table>
<p id="empty" hidden>No components match these filters.</p>
</div>
<h2 id="texts">Licence texts</h2>
<p>Canonical texts come from the SPDX License List. Where a package ships a licence that reads differently beyond its copyright lines, that text is reproduced separately under the licence it is closest to.</p>
{sections}{notices}<footer>
<p>Generated by <code>cargo xtask licenses</code> from <code>Cargo.lock</code>, <code>web/package-lock.json</code> and <code>docs/licenses/extra.toml</code> in the <a href="{REPO}">handbook repository</a>. Licensing questions: <a href="mailto:license@andrii.ai">license@andrii.ai</a>.</p>
</footer>
</main>
<script>
(()=>{{const rows=[...document.querySelectorAll('#bom tbody tr')],q=document.getElementById('q'),eco=document.getElementById('eco'),count=document.getElementById('count'),btns=[...document.querySelectorAll('[data-filter-scope]')],text=rows.map(r=>r.textContent.toLowerCase());let scope='';
function apply(){{const t=q.value.trim().toLowerCase(),e=eco.value;let n=0;rows.forEach((r,i)=>{{const show=(!t||text[i].includes(t))&&(!scope||r.dataset.scope.split(' ').includes(scope))&&(!e||r.dataset.eco===e);r.hidden=!show;if(show)n++}});document.getElementById('empty').hidden=n>0;count.textContent=n===rows.length?rows.length+' components':n+' of '+rows.length+' components'}}
btns.forEach(b=>b.addEventListener('click',()=>{{scope=b.dataset.filterScope;btns.forEach(x=>x.setAttribute('aria-pressed',String(x===b)));apply()}}));
q.addEventListener('input',apply);eco.addEventListener('change',apply);
function reveal(){{const el=location.hash&&document.getElementById(decodeURIComponent(location.hash.slice(1)));if(!el||el.tagName!=='TR')return;if(el.hidden){{q.value='';eco.value='';scope='';btns.forEach(x=>x.setAttribute('aria-pressed',String(x.dataset.filterScope==='')));apply()}}el.scrollIntoView({{block:'center'}});el.classList.add('hit');setTimeout(()=>el.classList.remove('hit'),2400)}}
addEventListener('hashchange',reveal);document.getElementById('controls').hidden=false;apply();reveal()}})();
</script>
</body>
</html>
"#,
        other = total - crates - npm,
        licence_count = licences.len(),
        sections = licence_sections(bom, lib),
    )
}

#[cfg(test)]
mod tests {
    use super::super::texts::tests::{ISC, MIT, library};
    use super::super::{FileKind, LicenseFile, Package, assemble, meta};
    use super::*;
    use std::collections::BTreeSet;

    fn bom() -> (Bom, Library) {
        let lib = library(&[("MIT", MIT), ("ISC", ISC), ("Apache-2.0", ISC)]);
        let pkg = |name: &str, license: &str, text: &str| Package {
            ecosystem: "npm".into(),
            name: name.into(),
            version: "1.0.0".into(),
            url: format!("https://github.com/x/{name}"),
            license: license.into(),
            scopes: BTreeSet::from([Scope::App, Scope::Test]),
            copyrights: Vec::new(),
            authors: vec!["A <a@example.test>".into()],
            files: vec![LicenseFile {
                kind: FileKind::License,
                text: text.into(),
            }],
            note: Some("a <note>".into()),
        };
        let bom = assemble(
            vec![
                pkg(
                    "left-pad",
                    "(MIT OR Apache-2.0)",
                    &MIT.replace("<year>", "2020"),
                ),
                pkg("<script>", "ISC", ISC),
            ],
            &lib,
        );
        (bom, lib)
    }

    #[test]
    fn page_is_deterministic_escaped_and_carries_hidden_stamps() {
        let (bom, lib) = bom();
        let a = page(&bom, &lib, "2026-11-04T12:00:00Z", "sha256:abc");
        let b = page(&bom, &lib, "2026-11-04T12:00:00Z", "sha256:abc");
        assert_eq!(a, b);
        assert_eq!(
            meta(&a, "bom:generated").as_deref(),
            Some("2026-11-04T12:00:00Z")
        );
        assert_eq!(meta(&a, "bom:inputs").as_deref(), Some("sha256:abc"));
        // The stamp lives only in <head>, never in visible text.
        let body = &a[a.find("<body>").unwrap()..];
        assert!(!body.contains("2026-11-04"));
        assert!(a.contains("&lt;script&gt;"));
        assert!(!a.contains("<td class=\"pkg\"><span class=\"name\"><script>"));
        assert!(a.contains("a &lt;note&gt;"));
        // One canonical MIT section, linked from the row.
        assert_eq!(a.matches("id=\"license-MIT\"").count(), 1);
        assert!(a.contains("(<a href=\"#license-MIT\">MIT</a> <span class=\"op\">OR</span>"));
    }

    #[test]
    fn short_urls_drop_scheme_and_www() {
        assert_eq!(short_url("https://www.sqlite.org/"), "sqlite.org");
        assert_eq!(short_url("http://x.test/a"), "x.test/a");
    }
}
