# Scaling recipes: established rules, exceptions, and what mise does

How food professionals change a recipe's yield, where the simple rule breaks, what 44,000 HelloFresh recipes
(each written for 2 and 4 servings) show in practice, and how mise applies it. The archive figures come from
every step amount published for both 2 and 4 servings: "×2" means it exactly doubles.

## The standard method: the conversion factor

Institutional food service (USDA, the Institute of Child Nutrition, state school-nutrition programs) and culinary
schools teach the same four steps:

1. **Factor** = servings wanted ÷ servings in the recipe (e.g. 30 ÷ 8 = 3.75).
2. **Convert** each amount to one unit, by weight where possible.
3. **Multiply** every ingredient by the factor.
4. **Round** to the largest sensible unit and a measurable amount ("minimal rounding is acceptable as long as the
   value is rounded up"; whole cans: "2 No. 10 cans").

They add one more rule: a **standardized recipe is tested at specific yields** (USDA quantity recipes at 50 and
100 servings), and a scaled recipe should be tested again before it is trusted.

**mise:** a published yield is used exactly; between published yields amounts are interpolated; outside them they
are scaled from the nearest one, rounded to kitchen measures (⅛ tsp, ½ TBSP, ¼ and ⅓ cup, ½ oz, half items),
and marked "≈". Derived servings are capped at 24.

## Exceptions the sources name

Wisconsin's school-nutrition guidance lists the four classic exceptions: "herbs and spices, leavening agents,
thickening agents, and liquids". Cooking time and equipment are the other two.

| Ingredient or factor | Established guidance | HelloFresh archive (2 → 4 servings) | mise |
|---|---|---|---|
| Proteins, produce, grains, pasta | Scale by the factor | ×2 in 100% of weighed/counted amounts | Factor |
| Fats and oils, butter | Scale by the factor | ×2 in 95% (5,465 amounts) | Factor |
| Milk, cream, coconut milk | Scale by the factor | ×2 in 92% | Factor |
| Herbs and spices | ICN: double from 50 → 100 servings, then **+25% per further 100 servings** and test; "heat builds" with cayenne, mustard, cloves, peppercorns. Blogs often say start at 75% and taste. | ×2 in 98% (3,260 seasonings and condiments) | Factor; the reduction applies to quantity cooking (hundreds of servings), not to household sizes |
| Salt | Same as spices; taste | ×2 (in the 98% above) | Factor |
| Leaveners (baking powder, soda, yeast) | Often ~75% of the factor above 2× (attributed to the CIA); Stella Parks (Serious Eats) scales linearly at home sizes | Few baked goods in the archive | Factor; flag for baking (future) |
| Thickeners (cornstarch, flour, roux) | Listed as non-proportional; adjust by consistency | Slurry water under ¼ cup: ×2 in 93% | Factor |
| **Cooking water, pan sauces** | Evaporation depends on the pan's surface, not the batch: same pan, same loss. ATK: doubled rice needs 3¾ cups water, not 4½ (¾ cup evaporates either way) | ¼ cup and up, in a pan or simmered: ×2 in only 35%; mean ×1.6. Most common shapes ×1.33 (¾ → 1 cup) and ×1.5 | **Proposed**: part of the water is fixed (see below) |
| Water for rice and grains | ATK: rice absorbs 1:1, plus a fixed evaporation amount | ×2 in 64% (½–1 cup: 96%) | Factor (decision: grains scale normally) |
| Water for soups, stews, pots | ATK: stews double fine if the pot holds them; reduce time longer | ×2 in 63% | Factor (decision: soups scale normally) |
| Cooking time and temperature | Never multiply. Same depth, same time; deeper or larger batches 10–50% longer; judge by look, not clock | Times unchanged between 2 and 4 servings | Step times kept as written |
| Pans and equipment | Use two pans or batches rather than crowding one (USDA: 2 skillets per 25 servings) | Notes such as "large pan for 4 servings", "work in batches" | Notes shown above the base yield |
| Whole items and packages | Round up to whole cans or packages | Counts and packages | Halves and quarters; package-aware shopping in phase 3 |

### Liquids: the proposed rule for pan sauces

Water added to a pan sauce, braise or stir-fry is partly **fixed** (what the open pan boils off) and partly
**per serving**. With a fixed share *f* of the water at the published yield *n₀*:

> water(*n*) = water(*n₀*) × ( *f* + (1 − *f*) × *n* ÷ *n₀* )

Fitted on 3,436 HelloFresh pan-sauce water amounts of ¼ cup or more, predicting the 4-serving amount from the
2-serving one:

| Rule | Exact after rounding | Within 15% | Mean error |
|---|---|---|---|
| Plain factor (double) | 43% | 46% | 24.6% |
| One fixed share (½) | 47% | 54% | 16.5% |
| **½ when it simmers or reduces in a pan or skillet, else ⅕** | **54%** | **67%** | 15.7% |

It applies only where mise has to extrapolate (sizes beyond what the recipe publishes, and single-yield recipes
such as web imports and your own), only to added water of ¼ cup or more, and never to rice, grains or soups.
Between published yields the published amounts already carry the shape. Small amounts (slurries, a splash) keep
the plain factor.

### Amounts never shrink

Not in the textbooks but found in the archive: some sources publish extra yields by formula (HelloFresh's 3 and
6 servings are ¾ and 1½ × the 4-serving amount, so 1 tsp oil "for 2 and 4" became ¾ tsp for 3). mise trusts the
yields the steps are written for first and derives any published amount that would get smaller as servings grow.
The one exception kept is the source's own step text: "½ cup water (⅓ cup for 4 servings)" appears 80 times
across five recipe families since 2019, deliberately: the water loosens a sauce base that doubles ("half the
marinara (all for 4)"), so the larger batch needs less added water.

## Sources

- Institute of Child Nutrition / NFSMI, *Adjusting a Recipe with Herbs and Spices* (No Time to Train, lesson 34):
  factor method; herbs and spices doubled to 100 servings then +25% per 100; "heat builds".
  <https://theicn.org/resources/354/no-time-to-trainshort-lessons-for-school-nutrition-assistants/105432/lesson-34-adjusting-a-recipe-with-herbs-and-spices.pdf>
- Wisconsin DPI, *Standardized Recipes in a Nutshell* (2023): scaling factor; herbs and spices, leavening,
  thickening agents and liquids may not scale proportionately.
  <https://dpi.wi.gov/sites/default/files/imce/school-nutrition/pdf/standardized_recipes.nutshell.pdf>
- America's Test Kitchen, *No, You Can't Always Just Double a Recipe*: same surface area, more liquid, longer
  evaporation; stews double if the pot is big enough.
  <https://www.americastestkitchen.com/articles/8047-no-you-cant-always-just-double-a-recipe>
- Cook's Illustrated, *The Science: The Secrets of Cooking Rice*: ¾ cup evaporates from single and double batches
  alike; doubled pilaf uses 3¾ cups water, not 4½.
  <https://www.americastestkitchen.com/cooksillustrated/how_tos/9395-science-the-secrets-of-cooking-rice>
- America's Test Kitchen, *Nailing the Perfect Ratio of Water to Rice*: rice absorbs water 1:1; the rest is
  evaporation, which depends on the pot, lid and heat.
  <https://www.americastestkitchen.com/articles/1692-nailing-the-perfect-ratio-of-water-to-rice>
- Aprovecho Research Center (Berick), *Heat Losses in a Cook Pot at Constant Temperature*: evaporation is the
  largest loss at a simmer (about 22% of 5 L per hour in a 25 cm pot); a lid recovers about 90% of it.
  <https://stoves.bioenergylists.org/node/528>
- Leavening and spice rules of thumb (75% above 2×; attributed to the CIA, not verified): Cooklang,
  <https://cooklang.org/blog/26-how-to-scale-recipes-without-mistakes/>; meez,
  <https://www.getmeez.com/blog/recipe-scaling-conversions-101>
- Institute of Child Nutrition, *Stir-Fry Pork* quantity recipe: 2 extra-large skillets for 25 servings, 4 for 50.
  <https://theicn.org/cnrb/pdfs/batch4_recipes_31-40_508c/Stir-Fry-Pork.25-50(multicultural)_508c.pdf>
