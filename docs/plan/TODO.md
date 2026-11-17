# Features not yet addressed

Agreed as worth doing, or open questions to settle, but not started. Each one starts with an ADR when the maintainer picks it up.

| Id | Item | Open question | Starts with |
|---|---|---|---|
| F1 | Runtime self-registration of turul services | Do turul-a2a and turul-mcp services add their own entries to the catalog at runtime, through the M19 `CatalogStore`, or is the catalog curated and deployed with the CLI? The store (M19, ADR-016) supports both, but who registers, when, under what authority, and with what removal or expiry is not decided. Covers the spec's #catalog-projection. | An ADR naming the registering services, the authority model, and entry lifecycle (add, refresh, remove). The work in turul-a2a and turul-mcp-framework goes in handoffs to those sessions. |
