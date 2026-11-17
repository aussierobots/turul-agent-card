# Features not yet addressed

Agreed as worth doing, or open questions to settle, but not started. Each one starts with an ADR when the maintainer picks it up.

| Id | Item | Open question | Starts with |
|---|---|---|---|
| F1 | Runtime self-registration of turul services | Do turul-a2a and turul-mcp services add their own entries to the catalog at runtime, through the M19 `CatalogStore`, or is the catalog curated and deployed with the CLI? The store (M19, ADR-016) supports both, but who registers, when, under what authority, and with what removal or expiry is not decided. Covers the spec's #catalog-projection. | An ADR naming the registering services, the authority model, and entry lifecycle (add, refresh, remove). The work in turul-a2a and turul-mcp-framework goes in handoffs to those sessions. |

Input received:
- F1, 2026-11-17, from GPS Trust (consumer, architecture ADR-0103): its catalog of 17 entries (6 MCP server cards, 11 A2A agent cards) is curated and deployed with the tooling. No GPS Trust service registers itself at runtime. Reasons: entries change only when a service is added, removed or moved; self-registration would need a start-up write and store permission on every service; and the spec's #catalog-projection needs an authority model that is not defined. This decides nothing for other turul services.
