# Draft spec issue: may a consumer fetch artifacts for display on an explicit user request?

Draft only; nothing here has been posted. Target: `github.com/Agent-Card/ai-catalog`, against `specification/ai-catalog.md` revision 2026-11-10.

---

**Title:** Clarify whether a consumer may dereference artifacts for name, description and version when a user explicitly asks

**Spec text.** Each of the three resolution sections ends with the same rule:

- [#resolving-an-artifact-s-display-name](https://ai-catalog.io/specification/#resolving-an-artifact-s-display-name): "A consumer SHOULD NOT dereference an artifact at render time solely to obtain a name. A registry, directory, or other service built on top of a catalog SHOULD resolve the name once at ingestion … and cache the result, rather than fetching artifacts on the rendering path."
- [#resolving-an-artifact-s-description](https://ai-catalog.io/specification/#resolving-an-artifact-s-description): the same rule for the description.
- [#resolving-an-artifact-s-version](https://ai-catalog.io/specification/#resolving-an-artifact-s-version): the same rule for the version.

Step 2 of each order uses "the referenced artifact's own canonical name / description / version, if the consumer has already fetched or cached the artifact".

At the same time, the `displayName` and `description` member definitions tell a publisher that, when the artifact carries the value (A2A Agent Card `name`/`description`, MCP Server Card `title`/`description`), the entry member "SHOULD be omitted to avoid duplicating a value that can drift out of sync". The version section says an entry carries `version` only to disambiguate a multi-version listing or to deliberately restate it.

**The gap.** A publisher that follows these SHOULDs publishes entries with only `identifier`, `type` and `url`. A consumer that follows the SHOULD NOT and is not an ingesting registry (a command-line listing, an editor plugin, a one-off inspection tool) then has no way to show a version or a description at all: it never fetched the artifact, so step 2 never applies, and step 3 gives "no version" and "no description". Both sides are conformant, and the user sees nulls for values every artifact carries.

The rationale for the SHOULD NOT (no fetching on the rendering path, resolve once and cache) fits a renderer that runs on every view. It is less clear that it is meant to cover a tool where the user explicitly asks, for one invocation, to look the values up: there the fetch is the requested action, not a side effect of rendering.

**Question.** Is a consumer that dereferences artifacts only when a user explicitly asks it to, for that request, within Safe Fetching limits, and that shows which values came from the artifact, covered by "SHOULD NOT dereference an artifact at render time solely to obtain" a value? If not, could the text say so, for example:

> A consumer MAY dereference referenced artifacts to resolve these values when a user explicitly requests it, observing Safe Fetching; this is resolution on request, not at render time.

If it is covered, could the text say which tools are expected to show versions and descriptions for catalogs that omit them as recommended, so that implementations do not each make their own exception?

**Context.** Raised by an implementation that follows the SHOULD NOT strictly and therefore shows no version or description for well-formed catalogs whose entries omit them as the spec recommends.
