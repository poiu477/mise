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
  { "decision": "merge", "a": ["seed:hellofresh-us", "<id>"], "b": ["seed:hellofresh-us", "<id>"],
    "by": "user", "note": "renamed: same recipe" },
  { "decision": "keep_apart", "a": ["seed:hellofresh-us", "<id>"], "b": ["seed:hellofresh-us", "<id>"], "by": "claude" }
]
```

`merge` joins the two records' dishes after drift splitting, so a split cannot undo it. `keep_apart` stops the pair
from being proposed for review again. `by` is `user` or `claude` (decided on the person's behalf, from their
earlier decisions); either can be overridden by editing the file and applying it again.
