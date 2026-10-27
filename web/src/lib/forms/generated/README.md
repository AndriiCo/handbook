# Generated form templates — do not hand-edit.

Each `<authority>__<code>.json` here is a byte-identical copy of the
latest `forms/<authority>/<code>/v<n>.json` (double underscore:
slugs and codes never contain `_`). Routes import these statically
so per-form data compiles into the release (form-framework.md §5).

Refresh: `cargo xtask form-scaffold --authority <a> --code <c>`
re-copies after `v<n>.json` changes. Verify:
`cargo xtask form-scaffold --check --authority <a> --code <c>`.
