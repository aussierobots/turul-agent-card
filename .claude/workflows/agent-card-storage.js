export const meta = {
  name: 'agent-card-storage',
  description: 'Implement turul-ai-catalog-storage (M19, ADR-016): trait + in-memory, sqlite, postgres, dynamodb, then the Lambda switch; one sonnet agent per slice, then the pre-push review',
  whenToUse: 'After ADR-016 exists. args.start: first slice id to run (default ST1); args.review: false to skip the final review',
  phases: [
    { title: 'Check', detail: 'haiku: ADR-016 present, M19 recorded', model: 'haiku' },
    { title: 'Code', detail: 'sonnet: one agent per slice; tests from ADR-016 first; gates run by the agent', model: 'sonnet' },
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
const BASE = `Repo turul-agent-card (cwd). Contract: docs/adr/ADR-016-catalog-storage.md (read it first). Decisions: docs/plan/DECISIONS.md (M19 approves this work; M6 keeps the format crate free of runtime deps). Spec: target/upstream/ai-catalog/specification/ai-catalog.md. Pattern to follow (read only, never edit): ~/turul-mcp-framework/crates/turul-mcp-task-storage (traits.rs, in_memory.rs, sqlite.rs, postgres.rs, dynamodb.rs, parity_tests.rs, Cargo.toml features). Rules: every dependency is workspace = true, with versions in the root [workspace.dependencies]. No ADR, plan or slice ids in code comments. Never weaken a test to make it pass; if ADR-016 is wrong against the spec, fix the ADR text and add a revision-log row. Do not bump versions or edit CHANGELOG. Do not commit.`
const SLICES = [
  ['ST1', 'Create crate crates/turul-ai-catalog-storage as a workspace member. Add the CatalogStore trait, its error type, the record and ETag types, assembly (through turul-ai-catalog validate and to_canonical_vec, the existing owners), and the in-memory backend as the default feature. Write the backend-agnostic parity test suite, so each backend runs the same tests, plus a dependency_direction-style check that turul-ai-catalog gains no dependency.'],
  ['ST2', 'Add the sqlite backend behind feature `sqlite`. Its parity tests run in cargo test against an in-memory database, so no server is needed.'],
  ['ST3', 'Add the postgres backend behind feature `postgres`. Its parity tests are #[ignore] and read TEST_DATABASE_URL, as turul-mcp-task-storage does. It must compile and lint under --all-features.'],
  ['ST4', 'Add the dynamodb backend behind feature `dynamodb`, using the table and key shape ADR-016 names. Its parity tests in tests/dynamodb.rs OWN THEIR ENVIRONMENT. Setup: use TURUL_AI_CATALOG_DYNAMODB_ENDPOINT if set. Otherwise spawn DynamoDB Local from DYNAMODB_LOCAL_JAR (default ~/.cache/dynamodb-local/DynamoDBLocal.jar, java from JAVA_HOME, PATH or /opt/homebrew/opt/openjdk/bin/java, with -inMemory and a free port). Wait until it answers, then create a uniquely named table (pk S, sk S) through the SDK with dummy credentials. The test creates the table; the store never does. Teardown: a Drop guard deletes the table and kills the child process, even on panic. If neither the endpoint nor the jar and java are available, every test prints a skip reason and returns. The tests are #[ignore] so plain cargo test stays fast; the GATES command runs them with --include-ignored. DynamoDB Local is installed on this machine, so they must actually run and pass here. Update ADR-016 (the Implements: line and the parity section) to describe this harness instead of TURUL_AI_CATALOG_DYNAMODB_TABLE, with a revision-log row. Any test-only dependency is a workspace dev-dependency. It must compile and lint under --all-features.'],
  ['ST5', 'Switch writers and the server to the store as ADR-016 specifies. produce (or the owner ADR-016 names) writes through CatalogStore. The Lambda serves the assembled document, superseding the bundled-file path: delete what ADR-016 says is superseded, callers and tests included, with no compatibility path. The local server (--features local) and cargo lambda watch keep working with the in-memory backend. Update ADR-010 and the dependency_direction test to match ADR-016, and update README.md "Serving a catalog".'],
]

phase('Check')
const ok = await agent('In repo turul-agent-card: does docs/adr/ADR-016-catalog-storage.md exist and name a CatalogStore trait, and does docs/plan/DECISIONS.md contain an M19 row? Change nothing.', {
  label: 'precheck', phase: 'Check', model: 'haiku', effort: 'low',
  schema: { type: 'object', properties: { adr: { type: 'boolean' }, m19: { type: 'boolean' } }, required: ['adr', 'm19'] },
})
if (!ok?.adr || !ok?.m19) throw new Error('ADR-016 and DECISIONS M19 must exist before implementation.')

phase('Code')
const start = Math.max(0, SLICES.findIndex(([id]) => id === (A.start || 'ST1')))
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
await agent(`${BASE}\n\nSlices ${done.join(', ')} of the storage work are implemented. Patch-bump [workspace.package].version once, together with every intra-workspace [workspace.dependencies] version. Add ONE adopter-facing CHANGELOG.md entry in the existing style covering turul-ai-catalog-storage, its features, and the Lambda change; cite "see ADR-016". Run: ${GATES}. Return the new version and pass/fail.`, { label: 'release', phase: 'Release', model: 'sonnet' })
if (A.review === false) return { done }

phase('Review')
const FINDINGS = { type: 'object', properties: { verdict: { type: 'string', enum: ['approve', 'revise', 'block'] }, findings: { type: 'array', items: { type: 'object', properties: { severity: { type: 'string', enum: ['BLOCKER', 'HIGH', 'MEDIUM', 'LOW'] }, location: { type: 'string' }, claim: { type: 'string' }, evidence: { type: 'string' }, fix: { type: 'string' } }, required: ['severity', 'location', 'claim', 'evidence', 'fix'] } } }, required: ['verdict', 'findings'] }
const must = x => (x?.findings || []).filter(f => f.severity === 'BLOCKER' || f.severity === 'HIGH')
const ctx = `${BASE}\nScope: the storage change, i.e. the new crate turul-ai-catalog-storage and everything it changed (git status / git diff, untracked files included).`
const R = [
  { key: 'adr-review', opts: { agentType: 'adr-review', model: 'sonnet' }, p: 'Review the storage change per your instructions.' },
  { key: 'devils-advocate', opts: { model: 'opus', effort: 'high' }, p: 'Devil\'s advocate on the storage change. Prove it wrong:\n- backends that diverge, i.e. a parity test missing for a guarantee ADR-016 states;\n- an ETag or conditional write that can lose an update;\n- assembly that is non-deterministic or bypasses validate or to_canonical_vec;\n- a signed catalog served from mutable state against #catalog-snapshot-coverage;\n- a superseded bundle path still present;\n- a dependency leak into turul-ai-catalog;\n- an injection or key-shape bug in sqlite, postgres or dynamodb.\nConstruct inputs and run them. Verbatim evidence only.' },
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
