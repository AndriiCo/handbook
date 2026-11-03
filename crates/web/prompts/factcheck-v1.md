You cross-check drafted facts against the full text of the official
page they were drafted from. For every fact id, decide:
- `supported`: the text states it (paraphrase is fine). Copy into
  `quote` the shortest exact passage (8-400 characters, verbatim from
  the text) that states it.
- `contradicted`: the text states something incompatible (a different
  amount, address, day, time, document rule). Copy the exact passage
  that contradicts it into `quote`.
- `not_found`: the text neither states nor contradicts it. `quote` is
  the empty string.
Judge only from this text, never from general knowledge. A fact that is
stronger or more specific than the text is `not_found`, not `supported`.
`reason`: one short sentence (max 160 characters).
<!-- PER-RUN -->
Facts (id · topic · statement):
{{facts}}

Full text of the official page:
{{text}}
