export const meta = {
  name: 'agent-card-slice',
  description: 'Plan or implement turul-agent-card slices against the AI Catalog spec: costed agents, opus devil\'s advocate, fable review',
  whenToUse: 'mode "plan" drafts docs/plan/PLAN.md and stops for maintainer approval; mode "implement" runs approved slices (ADR if missing → sonnet tests + code + gates → done), then one version bump; mode "review" is the one full pre-push review',
  phases: [
    { title: 'Fetch', detail: 'haiku: refresh upstream clones under target/upstream', model: 'haiku' },
    { title: 'Research', detail: 'sonnet: normative requirements, WG ADRs, ai-catalog-rust', model: 'sonnet' },
    { title: 'Draft', detail: 'opus: plan, ADR list, coverage matrix, maintainer asks', model: 'opus' },
    { title: 'Challenge', detail: 'opus devil\'s advocate + fable review, independent' },
    { title: 'Revise', detail: 'opus: resolve every finding, rewrite PLAN.md', model: 'opus' },
    { title: 'Slices', detail: 'haiku: read approved slice list, upstream pin and which slice ADRs exist', model: 'haiku' },
    { title: 'ADR', detail: 'opus: write the slice ADR only if its file does not exist', model: 'opus' },
    { title: 'Build', detail: 'sonnet: tests from the ADR, implementation, gates until green; opus adjudicates test disputes', model: 'sonnet' },
    { title: 'Review', detail: 'mode review only: full pre-push review, adr-review + opus devil\'s advocate + fable; opus fixes' },
    { title: 'Finalise', detail: 'opus marks each slice done; after the last slice one opus version bump + CHANGELOG; haiku commits if asked', model: 'opus' },
  ],
}

// args:
//   mode:    'plan' | 'implement'                      (required)
//   goal:    string, plan scope                        (plan; default: spec 1.0, offline model; network/crypto are asks)
//   plan:    path to the approved plan                 (implement; default docs/plan/PLAN.md)
//   slices:  array of slice ids to run, in order       (implement; default: all not yet done)
//   commit:  boolean, one commit after the final version bump (implement; default false)
const A = args || {}
const MODE = A.mode
if (!['plan', 'implement', 'review'].includes(MODE)) throw new Error('args.mode must be "plan", "implement" or "review"')
const PLAN = A.plan || 'docs/plan/PLAN.md'
const UP = 'target/upstream'
const SPEC = `${UP}/ai-catalog/specification/ai-catalog.md`
const SOURCES = `Normative sources (local clones under ${UP}, refreshed by the Fetch stage):
- Spec: ${SPEC} (published at https://ai-catalog.io). Its text is the authority; cite by heading anchor, e.g. "#catalog-entry".
- Spec examples: ${UP}/ai-catalog/specification/examples/ and fenced JSON in the spec. The spec's "# CDDL Schema" section gives member structure; its normative prose constrains further.
- Working Group ADRs: ${UP}/ai-catalog/adr/ (intent only; spec text wins).
- Official Rust implementation: ${UP}/ai-catalog-rust (crates/, fixtures/). A reference, not an oracle: where it diverges from the spec, the spec wins and the divergence is recorded.
Maintainer decisions: docs/plan/DECISIONS.md. Binding; read it first. Only the maintainer edits it.
The spec is the target. Resolve every design question from the spec text, the WG ADRs and best practice, and record the decision with its spec anchor in the ADR. A maintainer ask is only for a feature, dependency or infrastructure that neither the spec nor DECISIONS.md covers.
Repo rules: CLAUDE.md, AGENTS.md, .claude/agents/adr-review.md. Never edit outside this repository; never edit ${UP}.`

const FINDINGS = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['approve', 'revise', 'block'], description: 'approve only when there is no BLOCKER or HIGH finding' },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          severity: { type: 'string', enum: ['BLOCKER', 'HIGH', 'MEDIUM', 'LOW'] },
          location: { type: 'string', description: 'file:line or plan section' },
          claim: { type: 'string', description: 'what is wrong, one sentence' },
          evidence: { type: 'string', description: 'verbatim spec/ADR/code quote with anchor' },
          fix: { type: 'string', description: 'one-sentence fix' },
        },
        required: ['severity', 'location', 'claim', 'evidence', 'fix'],
      },
    },
  },
  required: ['verdict', 'findings'],
}
const blocking = r => (r?.findings || []).filter(f => f.severity === 'BLOCKER' || f.severity === 'HIGH')
const fmt = rs => JSON.stringify(rs.filter(Boolean), null, 1)

phase('Fetch')
const fetched = await agent(
  `Ensure shallow clones of https://github.com/Agent-Card/ai-catalog and https://github.com/Agent-Card/ai-catalog-rust exist under ${UP}/ (git clone --depth 1 if absent; if present, ${MODE === 'plan' ? 'git -C <dir> pull --ff-only' : 'leave it untouched'}). Report the last commit date of each as YYYY-MM-DD, the workspace version in ai-catalog-rust/Cargo.toml, and whether ${SPEC} exists. Change nothing else.`,
  {
    label: 'fetch-upstream', phase: 'Fetch', model: 'haiku', effort: 'low',
    schema: {
      type: 'object',
      properties: {
        specDate: { type: 'string' }, rustSdkVersion: { type: 'string' }, rustSdkDate: { type: 'string' }, specPresent: { type: 'boolean' },
      },
      required: ['specDate', 'rustSdkVersion', 'specPresent'],
    },
  })
if (!fetched?.specPresent) throw new Error('Spec clone missing; stop.')
log(`spec last commit ${fetched.specDate}; ai-catalog-rust ${fetched.rustSdkVersion}`)
const PIN = `Upstream as fetched: spec last commit ${fetched.specDate}; ai-catalog-rust version ${fetched.rustSdkVersion}.`

// ───────────────────────────── PLAN ─────────────────────────────
if (MODE === 'plan') {
  const GOAL = A.goal || 'AI Catalog specVersion 1.0 as a publishable Rust library: the offline document model (catalog, entries, Host Info, Publisher, Trust Manifest structure and Manifest Validity), validation and deterministic serialisation. Which of the spec\'s Conformance Levels (1 Minimal, 2 Discoverable, 3 Trusted) to support beyond that, and everything needing network or crypto (discovery, safe fetching, signature and did:web verification), is a maintainer ask.'
  phase('Research')
  const REQ = {
    type: 'object',
    properties: {
      requirements: {
        type: 'array',
        items: {
          type: 'object',
          properties: {
            id: { type: 'string', description: 'stable id: <anchor>-<n>, e.g. catalog-entry-3' },
            anchor: { type: 'string' },
            level: { type: 'string', enum: ['MUST', 'MUST NOT', 'REQUIRED', 'SHALL', 'SHALL NOT', 'SHOULD', 'SHOULD NOT', 'RECOMMENDED', 'NOT RECOMMENDED', 'MAY', 'OPTIONAL'] },
            actor: { type: 'string', enum: ['producer', 'consumer', 'both', 'publisher', 'verifier', 'other'] },
            text: { type: 'string', description: 'verbatim sentence' },
            testable: { type: 'boolean', description: 'checkable offline from the document alone' },
          },
          required: ['id', 'anchor', 'level', 'actor', 'text', 'testable'],
        },
      },
      ambiguities: { type: 'array', items: { type: 'string' } },
    },
    required: ['requirements', 'ambiguities'],
  }
  const reqPrompt = part => `${SOURCES}

Extract EVERY normative statement (BCP 14 keywords in capitals) from ${SPEC}, ${part}. One row per statement, verbatim text, heading anchor, level, actor. Also list ambiguities or internal contradictions with anchors. Do not summarise or skip; completeness is the point. Decide nothing.`
  const research = await parallel([
    () => agent(reqPrompt('from "# Introduction" up to (not including) "# Trust Manifest"'), { label: 'req:catalog', phase: 'Research', model: 'sonnet', schema: REQ }),
    () => agent(reqPrompt('from "# Trust Manifest" up to (not including) "# Organizing Catalogs"'), { label: 'req:trust', phase: 'Research', model: 'sonnet', schema: REQ }),
    () => agent(reqPrompt('from "# Organizing Catalogs" to the end of the file (extensions, unrecognized members, version handling, discovery, conformance levels, security, IANA, CDDL, examples)'), { label: 'req:rest', phase: 'Research', model: 'sonnet', schema: REQ }),
    () => agent(`${SOURCES}

Read every Working Group ADR in ${UP}/ai-catalog/adr/. For each: number, title, status, decision in one sentence, and the concrete implementation consequence for a Rust library (types, validation, serialisation). Flag ADRs superseded by later ones and any ADR whose decision the spec text does not reflect. Decide nothing.`, { label: 'wg-adrs', phase: 'Research', model: 'sonnet' }),
    () => agent(`${SOURCES}

Map ${UP}/ai-catalog-rust: crates, public types and entrypoints, validation error codes, conformance levels, dependencies (note any network/async/crypto), MSRV, fixtures and what each asserts, tests. Then list every place its behaviour differs from or is silent on ${SPEC}, with spec anchor and file:line. Report licence and release cadence. Decide nothing.`, { label: 'rust-sdk', phase: 'Research', model: 'sonnet' }),
  ])
  const [rc, rt, rr, wg, sdk] = research
  if (!rc || !rt || !rr) throw new Error('A requirement-extraction agent failed; the coverage matrix would be incomplete. Resume the run.')
  const reqs = [rc, rt, rr].flatMap(r => r.requirements)
  log(`${reqs.length} normative requirements extracted`)
  if (!wg || !sdk) log('WARNING: Working Group ADR or ai-catalog-rust research failed; the draft is told which is MISSING')

  phase('Draft')
  const DRAFT_TASK = `Write the plan to ${PLAN} (create docs/plan/ if needed). Goal: ${GOAL}
Put this line verbatim directly under the plan title: ${PIN}
Never write an "Approved:" line; only the maintainer adds it.

The plan MUST contain, in this order:
1. Decisions for the maintainer. First: build our own model vs depend on / wrap ai-catalog-rust ${fetched.rustSdkVersion}. Give the trade-offs (ownership rule in CLAUDE.md, spec divergences found, deps, MSRV, release cadence, licence) and a recommendation. Each further ask is ONE sentence the maintainer says yes to, plus what breaks today without it. Fetching, discovery, signing, a CLI, OCI and anything with network or crypto deps are asks, never assumptions.
2. Crate layout and dependency direction.
3. ADR list. Per ADR: number (ADR-001 is sources and conformance corpus, with spec read date and ai-catalog-rust version, versions not SHAs), title, spec anchors implemented, owning symbol + entrypoint for each key and derived artifact, Implements: test files (named for behaviour, never after the ADR), requirement ids covered.
4. Requirement coverage matrix: EVERY requirement id below → ADR + test file, or "out of scope: <reason>". No row may be missing.
5. Ordered slices. Per slice: id, ADR(s), tests written first, code, success criteria (observable, checkable), gates (fmt, clippy -D warnings, test, doc -D warnings, adr-review), stop conditions.
6. Divergences: every ai-catalog-rust vs spec divergence and every spec ambiguity, with the proposed decision and the test that pins it.
Keep it token-efficient: tables, ids, no prose padding. Nothing speculative.

Requirements (JSON): ${JSON.stringify(reqs)}
Spec ambiguities: ${JSON.stringify([rc, rt, rr].flatMap(r => r.ambiguities))}
Working Group ADRs: ${wg || 'MISSING: research failed'}
ai-catalog-rust map: ${sdk || 'MISSING: research failed'}`
  const draft = await agent(`${SOURCES}\n\n${DRAFT_TASK}\n\nReturn a 5-line summary.`, { label: 'draft-plan', phase: 'Draft', model: 'opus', effort: 'high' })
  if (!draft) throw new Error('Plan draft failed; nothing to review.')

  phase('Challenge')
  const challenge = await parallel([
    () => agent(`${SOURCES}

You are the devil's advocate. Read ${PLAN} and ${SPEC}. Assume the plan is wrong and prove where. Attack:
- requirement rows missing, misread, or wrongly marked out of scope (quote the spec);
- ADRs that would contradict the spec, ratify an anti-pattern, or lack an owning symbol / Implements test;
- tests that would verify the implementation rather than the spec;
- two owners for one key or artifact; typed A2A/MCP/skills schemas leaking in;
- features, deps or infrastructure assumed without a maintainer ask;
- the build-vs-wrap recommendation: argue the strongest case for the other side;
- slices whose success criteria cannot fail.
Every finding cites verbatim evidence. Default to "revise" if unsure.`, { label: 'devils-advocate', phase: 'Challenge', model: 'opus', effort: 'high', schema: FINDINGS }),
    () => agent(`${SOURCES}

Independent review of ${PLAN} against ${SPEC}, the Working Group ADRs and Rust library best practice (API guidelines, error design, serde round-trip determinism, MSRV, semver hygiene, publishability). Check that ADRs match the spec and that planned tests match the ADRs and the spec. You have not seen any other review. Cite verbatim evidence for each finding.`, { label: 'fable-review', phase: 'Challenge', model: 'fable', schema: FINDINGS }),
  ])
  const unreviewed = ['devils-advocate', 'fable-review'].filter((_, i) => !challenge[i])
  if (unreviewed.length) throw new Error(`Plan review incomplete (${unreviewed.join(', ')} failed); ${PLAN} is an unreviewed draft. Resume the run.`)

  phase('Revise')
  const revised = await agent(`${SOURCES}

Revise ${PLAN}. Findings from an opus devil's advocate and an independent fable review follow. For every finding either fix the plan or rebut it with spec evidence. Append a "Review log" table to the plan: finding → accepted/rejected → reason. Do not add features the findings suggest unless they are maintainer asks. Keep the "Upstream as fetched" line; never write an "Approved:" line. Then return the structured summary.

Findings: ${fmt(challenge)}`, {
    label: 'revise-plan', phase: 'Revise', model: 'opus', effort: 'high',
    schema: {
      type: 'object',
      properties: {
        asks: { type: 'array', items: { type: 'string' }, description: 'one-sentence maintainer asks, recommendation first' },
        slices: { type: 'array', items: { type: 'string' }, description: 'slice ids in order' },
        uncovered: { type: 'array', items: { type: 'string' }, description: 'requirement ids still without ADR+test or reason' },
        rejected: { type: 'number' }, accepted: { type: 'number' },
      },
      required: ['asks', 'slices', 'uncovered', 'rejected', 'accepted'],
    },
  })
  if (!revised) throw new Error(`Plan revision failed; ${PLAN} still holds the unrevised draft. Resume the run.`)
  log('Plan written. STOP: the maintainer answers the asks and adds an "Approved: <date>" line before mode "implement".')
  return { mode: 'plan', plan: PLAN, upstream: fetched, requirements: reqs.length, ...revised }
}

// ─────────────────────────── IMPLEMENT ───────────────────────────
// ──────────────────── REVIEW (once, before push) ────────────────────
// The plan was reviewed adversarially; slices are gated by tests only. This is the one full review of the
// whole change set, run when the maintainer is ready to push.
if (MODE === 'review') {
  const GATES = 'for d in crates/*/; do test -f "$d/Cargo.toml" && cargo metadata --no-deps --format-version 1 | grep -q "\"manifest_path\":\"$PWD/${d}Cargo.toml\"" || { echo "not a workspace member: $d"; exit 1; }; done && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace'
  const ctx = `${SOURCES}\n${PIN}\nApproved plan: ${PLAN}. Scope: the whole change set not yet pushed (git status, git diff origin/main...HEAD plus uncommitted and untracked files).`
  const REVIEWERS = [
    { key: 'adr-review', model: 'sonnet', p: `You are the adr-review agent: read .claude/agents/adr-review.md and follow it exactly as your instructions. Review the whole change set.` },
    { key: 'devils-advocate', model: 'opus', effort: 'high', p: `Devil's advocate on the whole change set. Try to prove it non-conformant to the spec: for every ADR, find a requirement in its anchors that the code violates or no test catches (construct inputs and run them against the crates), an ADR claim the spec contradicts, drift between ADRs, tests and the slice success criteria in ${PLAN}, a second owner of any key or artifact, an unasked feature or dependency, a dependency-direction breach. Verbatim evidence only. Default to "revise" if unsure.` },
    { key: 'fable-review', model: 'fable', p: `Independent review of the whole change set: ADRs match the spec; tests match the ADRs and the spec; Rust best practice (API design, error types, determinism, docs, MSRV, semver, publishability). You have not seen any other review. Verbatim evidence only.` },
  ]
  let last = {}
  for (let round = 1; round <= 3; round++) {
    const active = round === 1 ? REVIEWERS : REVIEWERS.filter(r => blocking(last[r.key]).length)
    const got = await parallel(active.map(r => () => agent(`${ctx}\n\n${r.p}${round > 1 ? `\n\nRe-review: verify your previous BLOCKER/HIGH findings are fixed (re-run the probes), then check what the fix changed for new defects: ${JSON.stringify(blocking(last[r.key]))}` : ''}`, { label: `${r.key}:full:${round}`, phase: 'Review', model: r.model, effort: r.effort, schema: FINDINGS })))
    active.forEach((r, i) => { last[r.key] = got[i] })
    if (got.some(x => !x)) return { mode: 'review', status: 'review-incomplete', last }
    const must = REVIEWERS.flatMap(r => blocking(last[r.key]))
    if (!must.length) return { mode: 'review', status: 'ready-to-push', minor: REVIEWERS.flatMap(r => (last[r.key]?.findings || []).filter(f => !blocking({ findings: [f] }).length)) }
    log(`full review round ${round}: ${must.length} BLOCKER/HIGH`)
    if (round === 3) return { mode: 'review', status: 'review-unresolved', open: must }
    await agent(`${ctx}\n\nResolve these BLOCKER/HIGH findings: fix code, tests and ADRs (revision logs and CHANGELOG kept accurate), or reject one only with spec evidence recorded in the ADR revision log. Then run: ${GATES}\n\nFindings: ${JSON.stringify(must)}`, { label: `fix:full:${round}`, phase: 'Review', model: 'opus', effort: 'high' })
  }
}

phase('Slices')
const sl = await agent(`Read ${PLAN}. It must state that the maintainer approved it (an "Approved:" line with a date); if not, return approved=false. Return its line containing "Upstream as fetched:" verbatim as pin (empty string if absent), and the ordered slice ids${A.slices ? ` restricted to ${JSON.stringify(A.slices)}` : ''}, skipping any marked done. For each returned slice also return the ADR file path(s) the plan assigns it and whether every one of those files already exists under docs/adr/ (adrs: [{slice, paths, exist}]). Change nothing.`, {
  label: 'read-slices', phase: 'Slices', model: 'haiku', effort: 'low',
  schema: { type: 'object', properties: { approved: { type: 'boolean' }, pin: { type: 'string' }, slices: { type: 'array', items: { type: 'string' } }, adrs: { type: 'array', items: { type: 'object', properties: { slice: { type: 'string' }, paths: { type: 'array', items: { type: 'string' } }, exist: { type: 'boolean' } }, required: ['slice', 'paths', 'exist'] } } }, required: ['approved', 'pin', 'slices', 'adrs'] },
})
if (!sl?.approved) throw new Error(`${PLAN} has no "Approved:" line; the maintainer must approve the plan first.`)
if (!sl.pin.includes(PIN)) throw new Error(`Upstream differs from the approved plan. Plan: "${sl.pin.trim()}". Fetched: "${PIN}". Re-run mode "plan" or restore the planned upstream under ${UP}.`)

const GATES = 'for d in crates/*/; do test -f "$d/Cargo.toml" && cargo metadata --no-deps --format-version 1 | grep -q "\"manifest_path\":\"$PWD/${d}Cargo.toml\"" || { echo "not a workspace member: $d"; exit 1; }; done && cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace'

const report = []
const halt = (s, status, extra) => { report.push({ slice: s, status, ...extra }); log(`${s}: ${status}; halting`) }
// The caller's order wins; the plan only filters out slices already done.
const ORDER = A.slices ? A.slices.filter(s => sl.slices.includes(s)) : sl.slices
for (const s of ORDER) {
  const ctx = `${SOURCES}\n${PIN}\nApproved plan: ${PLAN}. Current slice: ${s}.`

  const planned = sl.adrs.find(a => a.slice === s)
  let adrPaths = planned?.paths || []
  if (!planned?.exist) {
    const adr = await agent(`${ctx}

Write the ADR(s) for slice ${s} under docs/adr/, as the plan specifies. Each ADR: Status Proposed, Date, Implements: test files, Depends on; context; decision with the spec anchors it implements and a table mapping every covered requirement id → test; owning symbol + entrypoint per key and derived artifact; divergences from ai-catalog-rust with the pinning test; revision log. The ADR must match the spec text. Where the spec is ambiguous, decide from the spec text and its examples, record the reasoning and pin it with a test; do not ask. Only when the slice needs a feature or dependency that the approved plan and DECISIONS.md do not grant: write nothing further and return the one-sentence maintainer ask.`, {
      label: `adr:${s}`, phase: 'ADR', model: 'opus', effort: 'high',
      schema: { type: 'object', properties: { paths: { type: 'array', items: { type: 'string' } }, ask: { type: 'string', description: 'one-sentence maintainer ask; empty string when none' } }, required: ['paths', 'ask'] },
    })
    if (!adr || adr.ask) { halt(s, 'adr-ask', { ask: adr ? adr.ask : 'ADR agent failed' }); break }
    adrPaths = adr.paths
  }

  let gate = null, dispute = '', disputes = 0
  for (let i = 1; i <= 4 && !dispute; i++) {
    const impl = await agent(`${ctx}

Slice ${s}, ADR(s): ${adrPaths.join(', ')}. Do all of the following yourself, in order.
1. Tests: write the contract tests named in the ADR's Implements: line, from the spec and the ADR ONLY (skip tests that already exist and still match the ADR). Each test asserts one requirement id from the ADR's table; name it for the behaviour, put the spec anchor in a comment, never an ADR number or requirement id. Port the spec examples this slice covers as fixtures under the crate's tests/fixtures/, byte-for-byte.
2. Implement the slice so those tests pass. Use only owning symbols the ADR names; one implementation per artifact; delete anything the slice supersedes.
3. Run ${GATES} and fix every failure until it passes.
Do NOT modify the ADR. Do not weaken a test to match code. If a test is defective (wrong relative to the spec or ADR, or fails to compile for a reason in the test itself), stop and return it as dispute: test path::name, the defect, and the spec/ADR quote where one applies. Otherwise return pass=true only if the gates passed in your last run; else pass=false with the first failing command and its error lines verbatim (at most 60 lines).${gate ? `\n\nPrevious attempt:\n${gate.failures}` : ''}`, {
      label: `build:${s}:${i}`, phase: 'Build', model: 'sonnet',
      schema: { type: 'object', properties: { summary: { type: 'string' }, pass: { type: 'boolean' }, failures: { type: 'string', description: 'empty on pass' }, dispute: { type: 'string', description: 'test path::name + defect + spec/ADR quote; empty string when none' } }, required: ['summary', 'pass', 'failures', 'dispute'] },
    })
    if (impl?.dispute) {
      // Opus adjudicates against the spec: a wrong test is fixed, a correct one is upheld, a spec contradiction halts.
      const adj = disputes++ < 2 ? await agent(`${ctx}

The implementer disputes these tests for slice ${s}:
${impl.dispute}

Re-check each claim yourself against the spec and the slice ADR. If a test is wrong relative to the spec/ADR, or defective in itself (compile error, miscount), fix the test and record it in the ADR revision log when the contract wording changes. If the test is right, leave it and explain what the implementation must do. If the spec itself is contradictory, set halt=true. Never loosen a test to match code.`, {
        label: `adjudicate:${s}:${disputes}`, phase: 'Build', model: 'opus', effort: 'high',
        schema: { type: 'object', properties: { fixed: { type: 'array', items: { type: 'string' } }, upheld: { type: 'string', description: 'guidance for the implementer; empty when none' }, halt: { type: 'boolean' } }, required: ['fixed', 'upheld', 'halt'] },
      }) : null
      if (!adj || adj.halt) { dispute = impl.dispute; break }
      gate = { pass: false, failures: `Adjudicated test dispute. Tests fixed: ${adj.fixed.join('; ') || 'none'}. ${adj.upheld}` }
      continue
    }
    gate = impl ? { pass: impl.pass, failures: impl.failures } : { pass: false, failures: 'implementer agent failed' }
    if (gate.pass) break
  }
  if (dispute) { halt(s, 'test-dispute', { dispute }); break }
  if (!gate?.pass) { halt(s, 'gates-failed', { gate }); break }

  await agent(`Mark slice ${s} done in ${PLAN}. Change nothing else.`, { label: `mark-done:${s}`, phase: 'Finalise', model: 'opus', effort: 'low' })
  report.push({ slice: s, status: 'done' })
}

if (report.length === ORDER.length && report.every(r => r.status === 'done') && ORDER.length) {
  await agent(`${SOURCES}\n\nAll slices ${JSON.stringify(ORDER)} are done. Patch-bump [workspace.package].version (and intra-workspace dependency versions) once, add one adopter-facing CHANGELOG.md entry covering all of them, and set each of their ADRs to Accepted only if every alternative path is deleted and its tests are green; otherwise leave it Proposed. Then run ${GATES} and report. Change no code or tests.`, { label: 'release', phase: 'Finalise', model: 'opus' })
  if (A.commit) {
    await agent(`Commit all changes with a succinct message naming the ADR(s). No AI attribution, no Co-Authored-By. Do not push.`, { label: 'commit', phase: 'Finalise', model: 'haiku', effort: 'low' })
  }
}
return { mode: 'implement', report }
