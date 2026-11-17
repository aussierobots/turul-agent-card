export const meta = {
  name: 'agent-card-docs',
  description: 'Bring comments and rustdoc up to Rust API Guidelines for publishing: one sonnet agent per crate, gates, one opus accuracy check',
  whenToUse: 'Before publishing to crates.io / docs.rs, or after a large API change',
  phases: [
    { title: 'Docs', detail: 'sonnet: one agent per crate (parallel; each edits only its own crate)', model: 'sonnet' },
    { title: 'Gates', detail: 'haiku: fmt, clippy (missing_docs now enforced), test, doc', model: 'haiku' },
    { title: 'Accuracy', detail: 'opus: do the new docs state what the code does and what the ADRs and spec say?', model: 'opus' },
  ],
}

const GATES = 'cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p turul-ai-catalog-storage --all-features --all-targets -- -D warnings && cargo clippy -p turul-ai-catalog-lambda --features local --all-targets -- -D warnings && cargo test --workspace && cargo test --doc --workspace && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --all-features'
const CRATES = [
  'turul-ai-catalog', 'turul-ai-catalog-client', 'turul-ai-catalog-storage', 'turul-ai-catalog-trust',
  'turul-ai-catalog-tui', 'turul-ai-catalog-cli', 'turul-ai-catalog-lambda',
]
const RULES = `Rules (Rust API Guidelines C-CRATE-DOC, C-EXAMPLE, C-FAILURE, C-LINK, C-METADATA; std documentation conventions):
- Crate root: the crate-level doc (the README, where lib.rs includes it) says what the crate is for, its feature flags, and a minimal example. Add \`#![warn(missing_docs)]\` to every library crate, and document every public item it flags.
- Every public item: a one-line summary sentence in the third person ("Parses …", "Returns …"), then detail only where it adds something. Public items in the storage crate's feature-gated modules count too.
- Fallible functions: a \`# Errors\` section listing when each error variant is returned. Functions that can panic: a \`# Panics\` section, or remove the panic. Unsafe code: a \`# Safety\` section (none expected).
- Examples: a \`# Examples\` doctest on the main entrypoints (parse, validate, to_canonical_vec, load, produce, CatalogStore methods through InMemoryCatalogStore, sign/verify where usable offline). Examples must run offline, use \`?\` with a hidden \`# Ok::<(), Box<dyn std::error::Error>>(())\`, and never call unwrap() in visible lines.
- Intra-doc links (\`[\`Report\`]\`, \`[\`validate\`](crate::validate::validate)\`) instead of plain backticks wherever the target is in scope. No broken links (cargo doc -D warnings).
- Spec citations: link the section, e.g. \`[#catalog-entry](https://ai-catalog.io/specification/#catalog-entry)\` or keep \`#anchor\` text where that is the established style. Never ADR, plan, decision or slice ids (ADR-…, AMB-…, RD-…, M…, D…, S…, T…) in code comments or rustdoc; CLAUDE.md forbids it.
- Comments: state the invariant or the reason, not the mechanics a reader can see. Delete commented-out code, stale comments, comments that restate the code, and review history. Keep \`//\` comments in tests short; each test's comment names the behaviour or spec rule it pins.
- Error types: Display messages are lowercase, have no trailing period, and do not repeat their source (C-GOOD-ERR); each error type implements std::error::Error and returns its source where one exists.
- Cargo.toml metadata for each published crate: description, readme, keywords (≤5), categories (valid crates.io slugs), documentation = "https://docs.rs/<name>", repository, license via the workspace. The storage crate gets \`[package.metadata.docs.rs] all-features = true\` and \`rustdoc-args = ["--cfg", "docsrs"]\`, plus \`#![cfg_attr(docsrs, feature(doc_auto_cfg))]\`, so docs.rs shows every backend and which feature enables it.
- Change NO behaviour. Signatures stay as they are, except to remove a panic you document as removed. If a public item's name or signature is poor, report it rather than change it.`

phase('Docs')
const done = await parallel(CRATES.map(c => () => agent(`Repo turul-agent-card (cwd). Bring crate ${c} (crates/${c}: src/, its README.md, its Cargo.toml) up to Rust documentation best practice before publishing to crates.io and docs.rs. Edit ONLY files inside crates/${c}/; other agents are editing the other crates in parallel. Read CLAUDE.md, and the ADRs that own the crate's symbols (docs/adr/), so the docs state the contract correctly.

${RULES}

Then run, for this crate only: cargo clippy -p ${c} --all-targets --all-features -- -D warnings && cargo test -p ${c} --all-features && cargo test --doc -p ${c} --all-features && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p ${c} --all-features. Fix until green. Return: the number of items documented, comments removed or rewritten, examples added, and any API naming issues you report but did not change.`, { label: `docs:${c}`, phase: 'Docs', model: 'sonnet' })))
log(`docs: ${done.filter(Boolean).length}/${CRATES.length} crates finished`)

phase('Gates')
let g = await agent(`Run exactly: ${GATES}. Report pass/fail and failing output verbatim (≤60 lines). Change nothing.`, { label: 'gates', phase: 'Gates', model: 'haiku', effort: 'low', schema: { type: 'object', properties: { pass: { type: 'boolean' }, failures: { type: 'string' } }, required: ['pass', 'failures'] } })
if (!g?.pass) {
  await agent(`Repo turul-agent-card (cwd). The workspace gates fail after the documentation pass. Fix the cause without changing behaviour or weakening any test:\n${g?.failures}\nThen run: ${GATES}`, { label: 'fix-gates', phase: 'Gates', model: 'sonnet' })
  g = await agent(`Run exactly: ${GATES}. Report pass/fail and failing output verbatim. Change nothing.`, { label: 'gates:2', phase: 'Gates', model: 'haiku', effort: 'low', schema: { type: 'object', properties: { pass: { type: 'boolean' }, failures: { type: 'string' } }, required: ['pass', 'failures'] } })
  if (!g?.pass) return { status: 'gates-failed', g, done }
}

phase('Accuracy')
const acc = await agent(`Repo turul-agent-card (cwd). A documentation pass just rewrote the rustdoc and comments across all crates (git diff). Check that the new text is TRUE: each doc claim, # Errors list, # Panics section and example matches what the code does, what the owning ADR (docs/adr/) says, and what the spec says (target/upstream/ai-catalog/specification/ai-catalog.md). Check also that no behaviour changed: the diff touches only comments, doc attributes, Cargo metadata and lint attributes, except documented panic removals. Fix any false or misleading doc text directly. Return the claims you corrected, and any non-doc code change you found.`, { label: 'accuracy', phase: 'Accuracy', model: 'opus', effort: 'high', schema: { type: 'object', properties: { corrected: { type: 'array', items: { type: 'string' } }, codeChanges: { type: 'array', items: { type: 'string' } } }, required: ['corrected', 'codeChanges'] } })
return { status: acc ? 'ready-to-commit' : 'accuracy-check-failed', done, acc }
