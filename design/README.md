# design/

Shared design tokens for the operator dashboard (`web/`) and the
companion (`sidecar/web/`). Each surface imports `tokens.css` from its
own `styles.css`; Vite resolves and inlines the CSS at build time so the
single-file dashboard bundle and the companion's static output stay
self-contained.

Edit `tokens.css` to change a value once and have it propagate across
both surfaces. The palette is rooted in the avatar's physical DNA
(coral cheek, warm face white, dark CoreS3 chassis) defined in
`crates/stackchan-core/src/palette.rs`.
