# Seed bundle format (`mise-seed/1`)

A seed bundle loads many recipes at once from any source: a personal archive, another app's export, a
cookbook you typed up. mise knows nothing about where a bundle came from; a separate converter produces it.

A bundle is a directory:

```
my-bundle/
  manifest.json      what the bundle is and where its media lives
  variants.ndjson    one recipe version per line
  foods.ndjson       optional: knowledge about foods (identity, ingredients, homemade versions, where to buy)
```

## manifest.json

```json
{
  "format": "mise-seed/1",
  "source": { "key": "seed:my-archive", "name": "My recipe archive" },
  "created": "2026-10-26T18:00:00Z",
  "media_root": "D:/Recipes"
}
```

`source.key` identifies the source across re-imports: importing a bundle with the same key again updates
that source's variants instead of duplicating them. `media_root` is where the relative media paths below
are resolved; media is copied into mise's own storage on import.

## variants.ndjson

One JSON object per line. Every field except `external_id`, `title` and `ingredients` is optional.

| Field | Type | Meaning |
| --- | --- | --- |
| `external_id` | string | Stable ID within the source |
| `url` | string | Where the recipe lives, if anywhere |
| `parent_external_id` | string | The record this one was copied from (lineage), if the source tracks it |
| `title`, `subtitle`, `description` | string | Description in Markdown |
| `content_type` | `recipe` \| `ready_meal` \| `add_on` \| `product` | Defaults to `recipe`. Imported variants of a type can be removed later with `mise prune --content-type <type>`; after a converter starts leaving records out, `mise prune --missing-from <bundle>` removes them |
| `published_at`, `updated_at` | RFC 3339 | |
| `meta` | object | Free-form: `cuisines`, `tags`, `difficulty`, `prep_time`, `total_time` (ISO 8601 durations), `nutrition` |
| `yields` | number[] | Serving counts the source publishes amounts for |
| `ingredients` | object[] | See below |
| `amounts` | object[] | `{key, servings, amount, unit}`; `amount`/`unit` may be null ("salt to taste") |
| `steps` | object[] | `{index, text, images: [media path]}` in order |
| `hero_image` | string | Media path relative to `media_root` |
| `raw` | object | The source's original record, kept verbatim for re-processing |

An ingredient: `{key, name, shipped, allergens: [string], note, source_food: {external_id, image}}`.
`key` is unique within the variant and is what `amounts` refer to; `name` is the text as the source wrote
it; `shipped` says whether the source supplies it (meal kits) or it comes from the pantry.

## foods.ndjson

One food per line; all of it lands as *proposed* knowledge for review.

```json
{
  "name": "Southwest Spice Blend",
  "parent_path": ["Spice blends", "Southwestern"],
  "kind": "proprietary",
  "aliases": [],
  "facts": [
    {"kind": "ingredients", "value": "Chili powder, cumin, garlic powder, onion powder, salt, oregano.",
     "tier": "brand_support", "source_ref": "support chat", "observed_at": "2026-10-26"},
    {"kind": "homemade", "value": {"amount": "1 tbsp", "ingredients": ["3/4 tsp chili powder", "..."], "method": "Mix."},
     "tier": "brand_support"},
    {"kind": "buy", "value": ["Taco or southwest seasoning"], "tier": "inferred"}
  ]
}
```

`kind` is one of `food`, `brand`, `variety`, `cut`, `form`, `proprietary`. Fact `kind` is one of
`ingredients`, `allergens`, `homemade`, `buy`, `description`, `packet_size`; `tier` is one of `label`,
`brand_support`, `regional_label`, `inferred`, `copycat`.
