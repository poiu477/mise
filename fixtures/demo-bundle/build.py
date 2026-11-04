"""Writes variants.ndjson next to this file: mise's own demo recipes, written for the project (no outside content).

Run `python fixtures/demo-bundle/build.py` after editing a recipe here, and commit both files.
"""
import json
from pathlib import Path

OUT = Path(__file__).with_name("variants.ndjson")


def recipe(eid, title, subtitle, description, yields, lines, steps, prep, parent=None, tags=()):
    """lines: (key, name, shipped, {servings: (amount, unit)})"""
    return {
        "external_id": eid,
        "parent_external_id": parent,
        "title": title,
        "subtitle": subtitle,
        "description": description,
        "content_type": "recipe",
        "published_at": "2026-11-01T12:00:00Z",
        "meta": {"tags": list(tags), "difficulty": 1, "prep_time": prep},
        "yields": yields,
        "ingredients": [{"key": k, "name": n, "shipped": s, "allergens": []} for k, n, s, _ in lines],
        "amounts": [
            {"key": k, "servings": sv, "amount": a, "unit": u}
            for k, _, _, by in lines
            for sv, (a, u) in by.items()
        ],
        "steps": [{"index": i + 1, "text": t} for i, t in enumerate(steps)],
    }


pasta = recipe(
    "demo-tomato-pasta", "Weeknight Tomato Pasta", "with Garlic and Parmesan",
    "A pantry pasta for busy nights: a quick tomato sauce, plenty of garlic and a handful of cheese.",
    [2, 4],
    [
        ("spaghetti", "Spaghetti", True, {2: (6, "ounce"), 4: (12, "ounce")}),
        ("tomatoes", "Crushed Tomatoes", True, {2: (1, "can"), 4: (2, "can")}),
        ("garlic", "Garlic", True, {2: (2, "clove"), 4: (4, "clove")}),
        ("onion", "Yellow Onion", True, {2: (1, "unit"), 4: (2, "unit")}),
        ("parmesan", "Parmesan", True, {2: (0.25, "cup"), 4: (0.5, "cup")}),
        ("flakes", "Red Pepper Flakes", True, {2: (0.06, "teaspoon"), 4: (0.12, "teaspoon")}),
        ("oil", "Olive Oil", False, {2: (1, "tablespoon"), 4: (2, "tablespoon")}),
        ("salt", "Salt", False, {2: (None, None), 4: (None, None)}),
    ],
    [
        "Bring a large pot of salted water to a boil. Peel and finely chop the onion. Mince the garlic.",
        "Heat 1 TBSP olive oil (2 TBSP for 4 servings) in a large pan over medium heat. Add the onion and cook until soft, 5-6 minutes. Add the garlic and red pepper flakes; cook 1 minute more.",
        "Stir in the crushed tomatoes and a pinch of salt. Simmer until slightly thickened, 8-10 minutes.",
        "Meanwhile, cook the spaghetti until al dente, 9-11 minutes. Reserve ½ cup cooking water (1 cup for 4 servings), then drain.",
        "Toss the spaghetti with the sauce, adding splashes of reserved water as needed. Serve topped with the Parmesan.",
    ],
    "PT25M", tags=["vegetarian", "quick"],
)

chicken = recipe(
    "demo-lemon-chicken", "Lemon Herb Chicken", "with a Garlic Pan Sauce",
    "Golden chicken with a bright, buttery pan sauce made in the same skillet.",
    [2, 4],
    [
        ("chicken", "Chicken Breasts", True, {2: (12, "ounce"), 4: (24, "ounce")}),
        ("lemon", "Lemon", True, {2: (1, "unit"), 4: (2, "unit")}),
        ("garlic", "Garlic", True, {2: (1, "clove"), 4: (2, "clove")}),
        ("parsley", "Parsley", True, {2: (0.25, "ounce"), 4: (0.25, "ounce")}),
        ("stock", "Chicken Stock Concentrate", True, {2: (1, "unit"), 4: (2, "unit")}),
        ("butter", "Butter", True, {2: (1, "tablespoon"), 4: (2, "tablespoon")}),
        ("oil", "Olive Oil", False, {2: (1, "tablespoon"), 4: (2, "tablespoon")}),
        ("salt", "Salt", False, {2: (None, None), 4: (None, None)}),
        ("pepper", "Black Pepper", False, {2: (None, None), 4: (None, None)}),
    ],
    [
        "Pat the chicken dry and season all over with salt and pepper. Zest and halve the lemon. Mince the garlic and chop the parsley.",
        "Heat 1 TBSP olive oil (2 TBSP for 4 servings) in a large pan over medium-high heat. Cook the chicken until browned and cooked through, 5-7 minutes per side. Set aside.",
        "Lower the heat to medium. Add the garlic and cook 30 seconds. Stir in the stock concentrate and ¼ cup water (⅓ cup for 4 servings); simmer until reduced by half, 2-3 minutes.",
        "Off the heat, stir in the butter, the lemon zest and a squeeze of lemon juice. Slice the chicken, spoon over the sauce and scatter with parsley.",
    ],
    "PT30M",
)

tacos_v1 = recipe(
    "demo-bean-tacos-1", "Black Bean Tacos", "with Lime Crema",
    "Smoky beans, crisp onion and a tangy lime crema, ready in twenty minutes.",
    [2, 4],
    [
        ("tortillas", "Flour Tortillas", True, {2: (6, "unit"), 4: (12, "unit")}),
        ("beans", "Black Beans", True, {2: (1, "can"), 4: (2, "can")}),
        ("onion", "Yellow Onion", True, {2: (0.5, "unit"), 4: (1, "unit")}),
        ("cheese", "Cheddar Cheese", True, {2: (0.5, "cup"), 4: (1, "cup")}),
        ("lime", "Lime", True, {2: (1, "unit"), 4: (1, "unit")}),
        ("crema", "Sour Cream", True, {2: (2, "tablespoon"), 4: (4, "tablespoon")}),
        ("spice", "Taco Spice Blend", True, {2: (1, "tablespoon"), 4: (2, "tablespoon")}),
        ("oil", "Olive Oil", False, {2: (1, "teaspoon"), 4: (2, "teaspoon")}),
    ],
    [
        "Finely chop the onion. Drain and rinse the beans. Halve the lime; in a small bowl, mix the sour cream with a squeeze of lime juice.",
        "Heat 1 tsp olive oil (2 tsp for 4 servings) in a pan over medium heat. Cook half the onion until soft, 3-4 minutes. Add the beans and taco spice; mash lightly and cook until warmed through.",
        "Warm the tortillas. Fill with the beans, then top with the cheese, the remaining onion and the lime crema.",
    ],
    "PT20M", tags=["vegetarian"],
)

# A later version of the same dish: Monterey Jack instead of Cheddar, and more lime.
tacos_v2 = recipe(
    "demo-bean-tacos-2", "Black Bean Tacos", "with Lime Crema",
    "Smoky beans, crisp onion and a tangy lime crema, now with melty Monterey Jack.",
    [2, 4],
    [
        ("tortillas", "Flour Tortillas", True, {2: (6, "unit"), 4: (12, "unit")}),
        ("beans", "Black Beans", True, {2: (1, "can"), 4: (2, "can")}),
        ("onion", "Yellow Onion", True, {2: (0.5, "unit"), 4: (1, "unit")}),
        ("cheese", "Monterey Jack Cheese", True, {2: (0.5, "cup"), 4: (1, "cup")}),
        ("lime", "Lime", True, {2: (1, "unit"), 4: (2, "unit")}),
        ("crema", "Sour Cream", True, {2: (2, "tablespoon"), 4: (4, "tablespoon")}),
        ("spice", "Taco Spice Blend", True, {2: (1, "tablespoon"), 4: (2, "tablespoon")}),
        ("oil", "Olive Oil", False, {2: (1, "teaspoon"), 4: (2, "teaspoon")}),
    ],
    [s["text"] for s in tacos_v1["steps"]],
    "PT20M", parent="demo-bean-tacos-1", tags=["vegetarian"],
)
tacos_v2["published_at"] = "2026-11-02T12:00:00Z"

rice = recipe(
    "demo-garlic-rice", "Garlic Rice Bowl", "with Scallions and Soy",
    "Fragrant garlic rice topped with scallions and a drizzle of soy, for one or two.",
    [2],
    [
        ("rice", "Jasmine Rice", True, {2: (0.75, "cup")}),
        ("garlic", "Garlic", True, {2: (2, "clove")}),
        ("scallions", "Scallions", True, {2: (2, "unit")}),
        ("soy", "Soy Sauce", True, {2: (1, "tablespoon")}),
        ("sesame", "Sesame Oil", True, {2: (1, "teaspoon")}),
        ("salt", "Salt", False, {2: (None, None)}),
    ],
    [
        "Mince the garlic. Thinly slice the scallions, keeping the whites and greens apart.",
        "Heat the sesame oil in a small pot over medium heat. Add the garlic and scallion whites; cook 1 minute. Stir in the rice, 1½ cups water and a pinch of salt. Bring to a boil, cover and simmer until tender, 15-18 minutes.",
        "Fluff the rice and divide between bowls. Drizzle with the soy sauce and top with the scallion greens.",
    ],
    "PT25M", tags=["vegetarian", "quick"],
)

lines = [json.dumps(r, ensure_ascii=False) for r in [pasta, chicken, tacos_v1, tacos_v2, rice]]
OUT.write_bytes(("\n".join(lines) + "\n").encode("utf-8"))
print(f"wrote {len(lines)} variants")
