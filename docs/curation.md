# Food curation files

`mise food apply <file>` runs a JSON array of edits in one transaction (all or nothing). The same edits are
available one at a time as `mise food <command>`. Every food an edit touches becomes yours (`origin = user`), so
re-running matching (`mise match-foods --reset`) keeps it. Keep your curation file next to your seed data so a
rebuilt database can be brought back to the same state.

| Op | Fields | Effect |
| --- | --- | --- |
| `add` | `name`, `parent`?, `kind`? (`food`, `brand`, `variety`, `cut`, `form`, `proprietary`), `aliases`? | Create a food |
| `set_parent` | `food`, `parent` | Make `food` a kind of `parent` |
| `unparent` | `food` | Make `food` top-level |
| `alias` | `food`, `alias` | Another name for the same food |
| `unrelate` | `food`, `other` | Remove a related link |
| `merge` | `from`, `into` | Same food: move lines, aliases, children, related links and facts to `into`; repeating it is harmless |
| `set` | `food`, `key`, `value` | A property, e.g. `default_flexibility`: `strict` |

Foods are found by name, any alias, or slug; plurals and brand marks don't matter.

Two properties set how a food's bare counts read, for sources that publish only "1 unit". `count_unit` is the word
(`packet`, `whole`, `crown`), and `count_size` is how much one is (`1.5 oz`), given only where the source publishes the
food both ways. Together they read "1 packet (1½ oz)". Without them the amount stays a bare number.

`store_section` places a food on the shopping list (`Produce`, `Dairy & Eggs`); a food without one uses its nearest
ancestor's, and falls under Other when none has one. The sections and their order are in `mise_core::shopping::SECTIONS`.

Kinds of a food share its shopping line while a recipe in the week asks for the food itself. `shop_as_one`: `true`
makes two kinds share a line even without one (peppers bought as "peppers"); a category never does.

Two ops say how a food's amounts add up on the shopping list. Kinds of the food use its measures when they have
none of their own.

| Op | Fields | Effect |
| --- | --- | --- |
| `density` | `food`, `volume`, `weight` | What a volume weighs (`"1 cup"`, `"3 oz"`): cups and ounces of it add up as one weight |
| `item_size` | `food`, `item`, `size` | How much one item is (`"clove"`, `"1 tsp"`; or `"onion"`, `"8 oz"`): counts and measures add up. Give a volume and a weight in two ops if both are known |

A density alone never turns "2 cloves" into spoons: only an item size does. Both are also `mise food density` and
`mise food item-size`.

```json
[
  {"op": "merge", "from": "Oil", "into": "Cooking Oil"},
  {"op": "set_parent", "food": "Olive Oil", "parent": "Cooking Oil"},
  {"op": "set", "food": "Olive Oil", "key": "default_flexibility", "value": "strict"},
  {"op": "add", "name": "Kerrygold Garlic & Herb Butter", "parent": "Garlic Herb Butter", "kind": "brand"}
]
```

## Dish decisions

`mise dish apply <file>` loads decisions about which recipe records belong in one dish, replacing those recorded
before; `mise group-dishes` honours them on every run. Records are named by source and id, so decisions survive
re-imports and a rebuilt database:

```json
[
  { "decision": "merge", "a": ["seed:my-archive", "<id>"], "b": ["seed:my-archive", "<id>"],
    "by": "user", "note": "renamed: same recipe" },
  { "decision": "keep_apart", "a": ["seed:my-archive", "<id>"], "b": ["seed:my-archive", "<id>"], "by": "claude" }
]
```

`merge` joins the two records' dishes after drift splitting, so a split cannot undo it. `keep_apart` stops the pair
from being proposed for review again. `by` is `user` or `claude` (decided on the person's behalf, from their
earlier decisions); either can be overridden by editing the file and applying it again.

`mise dish prefer <file>` loads how often each version has been had, replacing what was recorded before. A dish then
opens on the version had most, among those with steps, and those versions count for more when naming it:

```json
[
  { "record": ["seed:my-archive", "<id>"], "times": 14 }
]
```
