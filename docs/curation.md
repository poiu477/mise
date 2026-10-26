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
