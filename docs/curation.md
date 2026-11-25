# Food curation files

`mise food apply <file>` runs a JSON array of edits in one transaction (all or nothing). The same edits are
available one at a time as `mise food <command>`. Every food an edit touches becomes yours (`origin = user`), so
re-running matching (`mise match-foods --reset`) keeps it. Keep your curation file next to your seed data so a
rebuilt database can be brought back to the same state.

| Op | Fields | Effect |
| --- | --- | --- |
| `add` | `name`, `parent`?, `kind`? (`food`, `brand`, `variety`, `cut`, `form`, `proprietary`), `aliases`? | Create a food |
| `keep` | `food` | The food is right as it stands: it becomes yours, and matching stops asking about it |
| `set_parent` | `food`, `parent` | Make `food` a kind of `parent` |
| `unparent` | `food` | Make `food` top-level |
| `alias` | `food`, `alias` | Another name for the same food |
| `unrelate` | `food`, `other` | Remove a related link |
| `merge` | `from`, `into` | Same food: move lines, aliases, children, related links and facts to `into`; repeating it is harmless |
| `set` | `food`, `key`, `value` | A property, e.g. `default_flexibility`: `strict` |
| `brand` | `name` | A brand matching strips from ingredient names: `{"op": "brand", "name": "Old Bay"}` |
| `unbrand` | `name` | No longer a brand; matching leaves the word in names |
| `distinct` | `a`, `b` | Two look-alike names are different foods; review stops offering the pair |

Foods are found by name, any alias, or slug; plurals and brand marks don't matter.

### Near-duplicate foods

Matching gives every ingredient name it doesn't know a food of its own, so one food written two ways becomes two
foods: "Red Onions" and "Red Onion, Diced". After matching, the foods it made whose names are the same once
plurals are folded and the words that only say how a food was cut up or handled are dropped ("diced", "shredded",
"peeled", "cooked") are paired up, and each pair becomes a `food_duplicate` question in the review queue, the
surest first.

The pairing goes by those words, not by how alike the names read: scoring names by similarity alone offers
"Honey" and "Hot Honey", or "Sourdough Bread" and "Sourdough Bread Bowl", which are different foods. Words that
say *which* food it is are kept, so a colour, a variety or a form ("Green Bell Pepper", "Fresh Mozzarella") is
left to the `food_parent` question, which already files it as a kind of its base.

A pair is answered three ways, from the review page or by hand:

- **One food** (`merge`): the rarer spelling folds into the one more lines use.
- **Two foods** (`distinct`): they are different foods. Kept by name in `food_distinctions` (migration
  `0023_food_distinctions.sql`), not by food id, because `mise match-foods --reset` deletes the foods matching made
  and a rebuilt database makes them afresh. `mise food export` writes each pair back out, so the answer survives a
  rebuild. Also `mise food distinct "Red Onions" "Red Onion, Diced"`.
- **A kind of** (`set_parent`): one goes under the other.

Matching reports foods per 100 ingredient lines, so a jump in how many names the sources use shows up here.

### Brands

Matching makes a name with a brand in it a `brand` food, a kind of the food without the brand: "Old Bay Seasoning"
under "Seasoning". It knows a brand two ways: a word marked ® or ™ ("Zesty Brand™ Salsa"), and the brands in the
`brands` table, which catches them unmarked. mise ships 42 there (migration `0022_brands.sql`); `brand` adds one and
`unbrand` takes one away, a shipped one included. Case, marks and spacing don't matter. A brand's words also count as
narrowing when matching decides whether one food is a kind of another.

Matching reads the brands when it runs, so an edit counts from the next run: `mise match-foods --reset` redoes the
foods matching already made. Both are also `mise food brand` and `mise food unbrand`.

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

## Writing curation back out

`mise food export <file>` writes the hand edits a database holds as a curation file: the brands added (`brand`) and
shipped ones taken away (`unbrand`), the look-alike pairs told apart (`distinct`), every food that is yours
(`add`), names given by hand (`merge` when recipes use
the name, `alias` otherwise), where each of your foods sits (`set_parent` or `unparent`, parents first), properties
and measures. Applied to a database rebuilt from the same recipes, it brings the food model back; applied to the one
it came from, it changes nothing. Two edits leave nothing to read back, so keep them in a hand-written file:
`unrelate`, and `unparent` on a food from a bundle's knowledge.

`mise dish export <file>` writes the recorded dish decisions as a decisions file for `mise dish apply`.

## Answering the review queue

Importing leaves questions in `review_items`. An admin answers them on the web app's review page (Account, then
Review queue), each stored the way the commands here store it, so re-running a stage keeps it:

| Question | Accept | Reject | Something else |
| --- | --- | --- | --- |
| Food match: a name that could mean two foods ("pepper") | `keep` | | `merge` into the food it is, or `set_parent` |
| Food parent: "Garden Tomato" a kind of "Tomato"? | `set_parent` | `keep` | `set_parent` to another food, or `merge` |
| Dish merge: a copy unlike its original | a `merge` decision | a `keep_apart` decision | |

A dish decision shows once dishes are grouped again (`mise group-dishes`, then `mise derive-options`). Export after
answering to keep the decisions with your curation files.

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
