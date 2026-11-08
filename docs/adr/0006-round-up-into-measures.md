# 0006 · Scaled amounts round up into standard measures

Accepted, 2026-11-02 (status.md's Decisions; commit "Exact shopping totals in the household's weight units; recipes
round up"). The rules and their evidence are in [scaling-rules.md](../scaling-rules.md).

## Context

A recipe is published for a few serving counts. At any other count mise interpolates between the published yields
or scales from the nearest one, which gives amounts like 0.83 tsp or 1.37 TBSP that no one can measure. They have
to become something a cook has in the drawer: cups, spoons, a pinch, an ounce on the scale, half a lime. The
shopping list is a different job: it adds up what the week needs, and there a rounded amount per line would add up
wrong.

## Decision

`Quantity::measured()` ([quantity.rs](../../crates/mise-core/src/quantity.rs)) rounds a derived amount **up, never
down**:

- Spoons and cups become **one standard measure** when that is at most 5% over (`SIMPLER`), otherwise **the fewest
  measures that come closest**: "1 tsp + 1 pinch", "¼ cup + 1 TBSP".
- Weights, volumes by the millilitre and counts go up in steps that suit the unit: ½ oz (¼ below one ounce), ¼ lb,
  5 g or ml (1 below 20), half an item (a quarter below one), a whole pinch.
- "Never down" allows for one quirk: sources store some measures a hair over (⅔ cup as 0.67), so an amount within
  0.6% over a measure counts as that measure (`SLACK`).
- A derived amount never passes the next published one: 13.76 oz published for 2 and 4 servings reads 13¾ oz at 3,
  not 14 oz (`servings.rs`).
- Published yields are shown exactly as published, and the shopping list uses `exact()`, never `measured()`.

## Alternatives

- **Round to the nearest measure.** This is what mise did before 2026-11-02 (`Quantity::rounded()`, still used by
  the scaling report). It is closer on average, but it rounds down as often as up, so it often gives less than
  the scaled recipe calls for. Food-service guidance allows rounding only as long as the value is rounded up
  ([scaling-rules.md](../scaling-rules.md#the-standard-method-the-conversion-factor)).
- **Show the exact decimal ("0.83 tsp").** Honest, but unusable at the stove, and it reads as false precision.
- **Always the nearest single measure.** One spoon is simpler to read, but for amounts between measures it is either
  short or far over; allowing a second, smaller measure keeps every amount within reach of the target.
- **Weigh everything in grams.** Precise, but the household cooks in cups and spoons, and the source recipes are
  written that way.

## Consequences

- Scaled amounts are never short and are at most a little over. A fuzz test checks that a measured amount is never
  less than the amount it came from (`crates/mise-core/tests/fuzz.rs`, allowing for sources that store ⅔ as 0.67).
- The measure search tries every combination of cups, spoons and a pinch, which is most of the fuzz suite's 25
  seconds in a debug build.
- The scaling report's "matches the source" score counts every round-up as a miss, so it understates how well
  scaling works; replacing it is on the task list in [status.md](../status.md).
- Whether counts should round to halves (½ lime, today) or to whole items is still an open question in status.md.
