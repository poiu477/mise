# 0001 · Build our own app rather than adapt Mealie or Tandoor

Accepted, 2026-10. Recorded in the design document ("Architecture and deployment") and status.md's Decisions.

## Context

The household cooks the same dish in several versions: a protein swapped, a side changed, a recipe republished with
new amounts. mise's core idea is a **dish** with **versions** (`variants`), whose differences become toggles on one
page (`dish_options`), with a default version chosen by how often each was had (`preferred_versions`). The plan and
the shopping list then hold a chosen version at a serving count. See the data model in
[architecture.md](../architecture.md#data-model).

Mealie and Tandoor are mature self-hosted recipe managers that already have accounts, search, meal plans and
shopping lists. The question was whether to adapt one of them.

## Decision

Build our own application, because neither Mealie nor Tandoor models dishes with versioned variants, and reuse
established libraries and services for the parts that are solved: Postgres trigram search, argon2 and
openidconnect for sign-in, the image crate for photo sizes, openapi-fetch and openapi-typescript for the web client.

## Alternatives

- **Fork Mealie or Tandoor and add versions.** In both, the recipe is the unit everything else points at: meal
  plans, shopping lists, ratings. Putting a dish above recipes changes the centre of their data model, so a fork
  would diverge from upstream at its core and lose the main benefit of adapting, which is upstream's updates.
- **Use one as it is, with each version as its own recipe.** It runs today with no code, but the versions of a
  dish become unrelated entries: no toggles between them, no default version, no "you've had this one most", and
  search shows each dish several times. Those are the features the household wanted.
- **Keep one of them for planning and shopping, with mise for dishes only.** Two systems to run and keep in step,
  and the shopping list needs the chosen version's amounts at the chosen servings, which only mise would know.

## Consequences

- mise builds and maintains what the existing apps already have: accounts and single sign-on, households, the plan,
  the shopping list, offline use. That is most of the code in `crates/mise-api` and `web/`.
- The data model fits the problem: grouping, options and preferred versions are first-class tables, not
  conventions layered on recipes.
- Coming from those apps stays possible: importing Mealie and Tandoor exports is planned for Phase 4, and publishing
  a dish back to them as schema.org data is listed for later in the design document.
