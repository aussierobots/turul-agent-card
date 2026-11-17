export const meta = {
  name: 'agent-card-registration',
  description: 'Implement runtime registration (M26, ADR-017): provenance, expiry-aware assembly, Registrar, trust verifier; one sonnet agent per slice, then the pre-push review',
  whenToUse: 'After ADR-017 is approved. args.start: first slice id to run (default R1); args.review: false to skip the final review',
  phases: [
    { title: 'Check', detail: 'haiku: ADR-017 present, M26 recorded', model: 'haiku' },
    { title: 'Code', detail: 'sonnet: one agent per slice; tests from ADR-017 first; gates run by the agent', model: 'sonnet' },
    { title: 'Release', detail: 'sonnet: one version bump + CHANGELOG entry', model: 'sonnet' },
    { title: 'Review', detail: 'adr-review + opus devil\'s advocate + fable on the storage change; opus fixes' },
  ],
}

// One agent per slice, in order (they share the crate and the root Cargo.toml). Each writes the tests
// ADR-016 names, then the code, and runs GATES itself until green. Postgres follows turul-mcp-task-storage
// (#[ignore] unless TURUL_AI_CATALOG_POSTGRES_URL is set; always compiled and linted). DynamoDB parity tests
// set up and tear down their own DynamoDB Local and table (ADR-016), so the gate runs them for real.
const A = args || {}
const GATES = 'cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo clippy -p turul-ai-catalog-storage --all-features --all-targets -- -D warnings && cargo clippy -p turul-ai-catalog-lambda --features local --all-targets -- -D warnings && cargo test --workspace && cargo test -p turul-ai-catalog-storage --all-features && { test ! -f crates/turul-ai-catalog-storage/tests/dynamodb.rs || cargo test -p turul-ai-catalog-storage --features dynamodb --test dynamodb -- --include-ignored --nocapture; } && { test ! -f crates/turul-ai-catalog-storage/tests/postgres.rs || cargo test -p turul-ai-catalog-storage --features postgres --test postgres -- --include-ignored; } && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace'
const BASE = `Repo turul-agent-card (cwd). Contract: docs/adr/ADR-017-runtime-registration.md (read it first; it extends ADR-016, the store, and uses ADR-015 and ADR-004). Decisions: docs/plan/DECISIONS.md (M26 approves this work; M6 keeps the format crate free of runtime deps; M21 is the version-kind guard). Reviewer findings accepted for ADR-017 are recorded in its revision log. Spec: target/upstream/ai-catalog/specification/ai-catalog.md. Pattern to follow (read only, never edit): ~/turul-mcp-framework/crates/turul-mcp-task-storage (traits.rs, in_memory.rs, sqlite.rs, postgres.rs, dynamodb.rs, parity_tests.rs, Cargo.toml features). Rules: every dependency is workspace = true, with versions in the root [workspace.dependencies]. No ADR, plan or slice ids in code comments. Never weaken a test to make it pass; if ADR-016 is wrong against the spec, fix the ADR text and add a revision-log row. Do not bump versions or edit CHANGELOG. Do not commit.`
const SLICES = [
  ['R1', 'Slice R1 of ADR-017: provenance (Lease {expires_at, writer} | Curated, owner) stored beside each entry record and covered by its ETag, in all four backends, with the atomic other-mode refusal (new backend guarantee) and the schema and key changes ADR-017 names. import and produce write Curated. Extend the parity suite so every backend runs the new checks, including cross-mode refusal and concurrent lease refreshes.'],
  ['R2', 'Slice R2 of ADR-017: assembly with an injected clock. Expired leases are left out, and every omitted entry is reported with its reason; nothing is dropped silently. Update the Lambda SnapshotCache and handler wiring as ADR-017 specifies.'],
  ['R3', 'Slice R3 of ADR-017: the Registrar, RegistrationPolicy and WriterId, plus register (including refresh) and unregister, with the authority check order, allow-lists, distinct errors and every refusal ADR-017 lists.'],
  ['R4', 'Slice R4 of ADR-017: the publisher verifier trait in storage, implemented in turul-ai-catalog-trust behind its `registration` feature (verify_entry), with no crypto in storage and no dependency cycle. Add the feature to the gates and the docs.rs metadata as needed.'],
  ['R5', 'Slice R5 of ADR-017 (M27). First, if ADR-017 has no store-command section, add one: command shape, backend selection flags or env (the same vars as the Lambda: TURUL_AI_CATALOG_TABLE etc.), exit codes consistent with ADR-011, and requirement rows; add a revision-log row. Then implement the operator store command in the CLI, `turul-ai-catalog store`, at least `list` (entries with provenance and expiry state) and `remove` (by identifier and optional version), against the backend selected by flags or environment as ADR-017 specifies. All logic stays in the storage or client crates; the CLI only parses arguments, and the dependency_direction test is updated. Tests run against in-memory and sqlite.'],
]

phase('Check')
const ok = await agent('In repo turul-agent-card: does docs/adr/ADR-017-runtime-registration.md exist and name a Registrar, and does docs/plan/DECISIONS.md contain an M26 row? Change nothing.', {
  label: 'precheck', phase: 'Check', model: 'haiku', effort: 'low',
  schema: { type: 'object', properties: { adr: { type: 'boolean' }, m19: { type: 'boolean' } }, required: ['adr', 'm19'] },
})
if (!ok?.adr || !ok?.m19) throw new Error('ADR-017 and DECISIONS M26 must exist before implementation.')

phase('Code')
const start = Math.max(0, SLICES.findIndex(([id]) => id === (A.start || 'R1')))
const done = []
for (const [id, task] of SLICES.slice(start)) {
  let r = await agent(`${BASE}\n\nSlice ${id}: ${task}\nWrite the tests ADR-016's Implements: line and requirement table name for this slice first (skip any that already exist), then the code. Done when this passes: ${GATES}\nReturn a 2-line summary, or "DISPUTE: <test> <defect> <spec/ADR quote>" if a test is wrong, or "BLOCKED: <reason>".`, { label: `code:${id}`, phase: 'Code', model: 'sonnet' })
  if (r && /^DISPUTE/.test(r)) {
    await agent(`${BASE}\n\nThe implementer of ${id} disputes a test: ${r}\nRe-check it against ADR-016 and the spec. Fix the test if it is wrong, or leave it and write in your answer what the code must do. Never loosen a test to match code.`, { label: `adjudicate:${id}`, phase: 'Code', model: 'opus', effort: 'high' })
    r = await agent(`${BASE}\n\nSlice ${id}: ${task}\nA test dispute was adjudicated; re-read the tests. Finish the slice. Done when this passes: ${GATES}\nReturn a 2-line summary or "BLOCKED: <reason>".`, { label: `code:${id}:2`, phase: 'Code', model: 'sonnet' })
  }
  if (!r || /^(BLOCKED|DISPUTE)/.test(r)) return { done, blocked: id, reason: r }
  done.push(id)
}

phase('Release')
await agent(`${BASE}\n\nSlices ${done.join(', ')} of the registration work are implemented. Patch-bump [workspace.package].version once, together with every intra-workspace [workspace.dependencies] version. Add ONE adopter-facing CHANGELOG.md entry in the existing style covering runtime registration (Registrar, modes, expiry report, verifier feature) and the re-import requirement for existing stores; cite "see ADR-017". Run: ${GATES}. Return the new version and pass/fail.`, { label: 'release', phase: 'Release', model: 'sonnet' })
if (A.review === false) return { done }

phase('Review')
const FINDINGS = { type: 'object', properties: { verdict: { type: 'string', enum: ['approve', 'revise', 'block'] }, findings: { type: 'array', items: { type: 'object', properties: { severity: { type: 'string', enum: ['BLOCKER', 'HIGH', 'MEDIUM', 'LOW'] }, location: { type: 'string' }, claim: { type: 'string' }, evidence: { type: 'string' }, fix: { type: 'string' } }, required: ['severity', 'location', 'claim', 'evidence', 'fix'] } } }, required: ['verdict', 'findings'] }
const must = x => (x?.findings || []).filter(f => f.severity === 'BLOCKER' || f.severity === 'HIGH')
const ctx = `${BASE}\nScope: the registration change (ADR-017) and everything it changed (git status / git diff, untracked files included).`
const R = [
  { key: 'adr-review', opts: { agentType: 'adr-review', model: 'sonnet' }, p: 'Review the storage change per your instructions.' },
  { key: 'devils-advocate', opts: { model: 'opus', effort: 'high' }, p: 'Devil\'s advocate on the registration change. Prove it wrong:\n- a write that bypasses the allow-list or the signature requirement;\n- an identifier minted under another publisher (#catalog-projection);\n- a takeover, or a reassignment of an identifier;\n- a cross-mode write that lands;\n- a lease refresh race that shortens a lease or loses an update;\n- an expired entry served, or silently dropped with no report;\n- a clock read inside a pure function;\n- provenance missing from the ETag;\n- a backend that diverges from the parity suite;\n- crypto pulled into storage default features.\nConstruct inputs and run them. Verbatim evidence only.' },
  { key: 'fable-review', opts: { model: 'fable' }, p: 'Independent review of the storage change: ADR-016 matches the spec; the tests match ADR-016; Rust best practice for an async storage trait (Send + Sync bounds, error design, feature hygiene, connection handling, publishability). Verbatim evidence only.' },
]
let last = {}
for (let round = 1; round <= 3; round++) {
  const active = round === 1 ? R : R.filter(x => must(last[x.key]).length)
  const got = await parallel(active.map(x => () => agent(`${ctx}\n\n${x.p}${round > 1 ? `\n\nRe-review: verify your previous BLOCKER/HIGH findings are fixed, then check only what the fix changed: ${JSON.stringify(must(last[x.key]))}` : ''}`, { label: `${x.key}:${round}`, phase: 'Review', schema: FINDINGS, ...x.opts })))
  active.forEach((x, i) => { last[x.key] = got[i] })
  if (got.some(v => !v)) return { done, status: 'review-incomplete', last }
  const open = R.flatMap(x => must(last[x.key]))
  if (!open.length) return { done, status: 'ready-to-commit', minor: R.flatMap(x => (last[x.key]?.findings || []).filter(f => !must({ findings: [f] }).length)) }
  log(`storage review round ${round}: ${open.length} BLOCKER/HIGH`)
  if (round === 3) return { done, status: 'review-unresolved', open }
  await agent(`${ctx}\n\nResolve these BLOCKER/HIGH findings. Fix the code, tests and ADR-016, keeping the revision log and CHANGELOG accurate. Reject a finding only with spec evidence, recorded in the revision log. Then run: ${GATES}\n\nFindings: ${JSON.stringify(open)}`, { label: `fix:${round}`, phase: 'Review', model: 'opus', effort: 'high' })
}
