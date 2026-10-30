# Brand assets (operator-owned)

Drop the official provider artwork here, one file per provider id:

- `google.svg`
- `apple.svg`
- `meta.svg`

The login page picks these up automatically (`import.meta.glob`) and
shows one button with the official logo per provider listed by
`GET /api/v1/auth/providers`. Until a file lands, that provider's button
is text-only. Use each owner's current artwork per its brand guidelines.
