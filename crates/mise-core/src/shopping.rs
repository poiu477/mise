//! The shopping list: what several recipes need of one food, added up and worded for buying. Kinds of one food
//! can share a line ([`line_food`]), amounts in different units add up where the food says how they convert
//! ([`Measures`]), and ticks are kept per food so a line that merges or splits keeps them ([`foods_to_mark`]).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use uuid::Uuid;

use crate::quantity::{Family, PINCH_TSP, Quantity, Unit, decimal, format_exact, parse_number};
use crate::servings::CountUnit;

/// Store sections in the order a shop is walked. Foods with none go under `OTHER`.
pub const SECTIONS: &[&str] = &[
    "Produce",
    "Meat & Seafood",
    "Dairy & Eggs",
    "Bakery",
    "Pasta & Grains",
    "Canned & Jarred",
    "Sauces & Condiments",
    "Spices & Seasonings",
    "Baking",
    "Snacks & Nuts",
];
/// The section for foods that have none.
pub const OTHER: &str = "Other";
/// Pantry lines the recipes expect you to have: last, to check rather than buy.
pub const CHECK: &str = "Check you have";

/// Where a section sorts in the shop.
pub fn section_order(name: &str) -> usize {
    match name {
        OTHER => SECTIONS.len(),
        CHECK => SECTIONS.len() + 1,
        _ => SECTIONS
            .iter()
            .position(|s| *s == name)
            .unwrap_or(SECTIONS.len()),
    }
}

/// How a household reads weights on its shopping list, for all its foods or for one.
///
/// ```
/// use mise_core::quantity::{Quantity, Unit};
/// use mise_core::shopping::WeightUnit;
///
/// let grams = Quantity::new(30.0, Unit::Oz).in_unit(&Unit::G).unwrap();
/// assert_eq!(WeightUnit::LbOz.show(grams), "1 lb 14 oz");
/// assert_eq!(WeightUnit::Oz.show(grams), "30 oz");
/// assert_eq!(WeightUnit::Lb.show(grams), "1⅞ lb");
/// assert_eq!(WeightUnit::Metric.show(grams), "850 g");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WeightUnit {
    /// "1 lb 14 oz", and ounces under a pound.
    #[default]
    LbOz,
    /// Ounces only: "30 oz".
    Oz,
    /// Pounds only, in fractions where they fit: "1⅞ lb".
    Lb,
    /// Grams, and kilograms from one.
    Metric,
}

impl WeightUnit {
    /// Every choice, so a stored name can be looked up.
    pub const ALL: [WeightUnit; 4] = [
        WeightUnit::LbOz,
        WeightUnit::Oz,
        WeightUnit::Lb,
        WeightUnit::Metric,
    ];

    /// The name it is stored under: "lb_oz", "oz", "lb", "metric".
    pub fn as_str(self) -> &'static str {
        match self {
            WeightUnit::LbOz => "lb_oz",
            WeightUnit::Oz => "oz",
            WeightUnit::Lb => "lb",
            WeightUnit::Metric => "metric",
        }
    }

    /// The choice a stored name stands for ("lb_oz" is `LbOz`); `None` for a name it doesn't know.
    pub fn parse(s: &str) -> Option<WeightUnit> {
        WeightUnit::ALL.into_iter().find(|w| w.as_str() == s)
    }

    /// For a package's size: pounds read as pounds and ounces, so a 1 oz packet isn't "0.06 lb".
    fn for_size(self) -> WeightUnit {
        if self == WeightUnit::Lb {
            WeightUnit::LbOz
        } else {
            self
        }
    }

    /// The weight in these units, exactly.
    pub fn show(self, grams: f64) -> String {
        let oz = grams
            / Quantity::new(1.0, Unit::Oz)
                .in_unit(&Unit::G)
                .unwrap_or(1.0);
        match self {
            WeightUnit::Oz => Quantity::new(oz, Unit::Oz).exact(),
            WeightUnit::Lb => Quantity::new(oz / 16.0, Unit::Lb).exact(),
            WeightUnit::LbOz => {
                let lb = ((oz + 0.005) / 16.0).floor();
                let rest = oz - lb * 16.0;
                match (lb > 0.0, rest >= 0.005) {
                    (false, _) => Quantity::new(oz, Unit::Oz).exact(),
                    (true, false) => format!("{lb} lb"),
                    (true, true) => format!("{lb} lb {}", Quantity::new(rest, Unit::Oz).exact()),
                }
            }
            WeightUnit::Metric if grams >= 1000.0 => format!("{} kg", decimal(grams / 1000.0)),
            WeightUnit::Metric => format!("{} g", grams.round().max(1.0)),
        }
    }
}

/// The total of `needs` in words, exactly, for a food with no density or item sizes recorded: see
/// [`Measures::total`].
pub fn total(needs: &[Quantity], count: Option<&CountUnit>, weights: WeightUnit) -> Option<String> {
    Measures::default().total(needs, count, weights)
}

/// How much one item of a food is ("clove", "onion"): a volume, a weight, or both.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemSize {
    /// The word recipes count it in, singular: "clove".
    pub item: String,
    /// Its volume in milliliters, chopped or minced as recipes measure it: one onion is about a cup.
    pub ml: Option<f64>,
    /// Its weight in grams.
    pub grams: Option<f64>,
}

/// What a food's amounts convert by on the shopping list, beyond the units themselves: what a volume of it
/// weighs (its density), and how much one clove, onion or packet of it is. Recorded per food in `food_measures`.
/// Without them a volume never becomes a weight, and a density alone never turns "2 cloves" into tablespoons.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measures {
    /// Grams in one milliliter: grated Parmesan's "1 cup weighs 3 oz" is about 0.36.
    pub grams_per_ml: Option<f64>,
    /// How much one of each item is.
    pub items: Vec<ItemSize>,
}

/// A food's amounts added up, before they are worded.
struct Sum {
    tsp: f64,
    grams: f64,
    /// Counted items by their word ("" for a bare number), in the order they came.
    counted: Vec<(String, f64)>,
}

impl Sum {
    fn parts(&self) -> usize {
        usize::from(self.tsp > 0.0) + usize::from(self.grams > 0.0) + self.counted.len()
    }
}

/// Teaspoons in a milliliter.
fn tsp_per_ml() -> f64 {
    Quantity::new(1.0, Unit::Ml)
        .in_unit(&Unit::Tsp)
        .unwrap_or(0.2029)
}

impl Measures {
    /// Records that `ml` milliliters of the food weigh `grams` grams, unless a density is recorded already: the
    /// first one added wins, so a food's own comes before its parent's.
    pub fn add_density(&mut self, ml: f64, grams: f64) {
        if self.grams_per_ml.is_none() && ml > 0.0 && grams > 0.0 {
            self.grams_per_ml = Some(grams / ml);
        }
    }

    /// Records how much one `item` is, from a volume or a weight such as "1 cup" or "8 oz". A size already
    /// recorded for that item and kind of measure stays, so a food's own comes before its parent's. Counts and
    /// pinches say nothing about size and are ignored.
    pub fn add_item(&mut self, item: &str, size: &Quantity) {
        let (ml, grams) = match size.unit.family() {
            Family::Volume => (size.in_unit(&Unit::Ml), None),
            Family::Weight => (None, size.in_unit(&Unit::G)),
            Family::Count | Family::Pinch => return,
        };
        if item.is_empty() || size.amount <= 0.0 {
            return;
        }
        match self.items.iter_mut().find(|i| i.item == item) {
            Some(known) => {
                known.ml = known.ml.or(ml);
                known.grams = known.grams.or(grams);
            }
            None => self.items.push(ItemSize {
                item: item.to_string(),
                ml,
                grams,
            }),
        }
    }

    /// Everything `other` records that this doesn't: a line converts by every food on it, and a food by its
    /// parent's measures after its own.
    pub fn add(&mut self, other: &Measures) {
        if let Some(d) = other.grams_per_ml {
            self.add_density(1.0, d);
        }
        for i in &other.items {
            if let Some(ml) = i.ml {
                self.add_item(&i.item, &Quantity::new(ml, Unit::Ml));
            }
            if let Some(g) = i.grams {
                self.add_item(&i.item, &Quantity::new(g, Unit::G));
            }
        }
    }

    fn item(&self, item: &str) -> Option<&ItemSize> {
        self.items.iter().find(|i| i.item == item)
    }

    /// One `item` in teaspoons, from its volume, or from its weight where the density converts it.
    fn item_tsp(&self, item: &str) -> Option<f64> {
        let i = self.item(item)?;
        let ml = i.ml.or_else(|| Some(i.grams? / self.grams_per_ml?))?;
        Some(ml * tsp_per_ml())
    }

    /// One `item` in grams, from its weight, or from its volume where the density converts it.
    fn item_grams(&self, item: &str) -> Option<f64> {
        let i = self.item(item)?;
        i.grams.or_else(|| Some(i.ml? * self.grams_per_ml?))
    }

    /// `a` of `from` in `to` items, through a measure both sizes are known in: 1 head of garlic in cloves.
    fn items_in(&self, a: f64, from: &str, to: &str) -> Option<f64> {
        if from == to {
            return Some(a);
        }
        let by_volume = self.item_tsp(from).zip(self.item_tsp(to));
        let by_weight = self.item_grams(from).zip(self.item_grams(to));
        by_volume.or(by_weight).map(|(f, t)| a * f / t)
    }

    /// These measures with a count unit's own size ("1 packet" is 2 tsp) as an item size, after any recorded.
    fn with_count(&self, count: Option<&CountUnit>) -> Measures {
        let mut m = self.clone();
        if let Some((c, size)) = count.and_then(|c| Some((c, c.size.as_ref()?))) {
            m.add_item(&c.noun, size);
        }
        m
    }

    /// `needs` added up. Amounts are converted only to add them up, never to restate an amount the recipes
    /// wrote one way; so a line with one kind of amount reads as written. Where there is more than one, in turn:
    ///
    /// 1. Everything goes into the line's count when it all converts to it: "1 onion" and "½ cup" of onion are
    ///    "1½ onions" when an onion is a cup. A shopper buys whole items.
    /// 2. Otherwise counted items whose size is known fold into the measure there is of them: "1 packet" and
    ///    "1½ oz" are "3 oz" next to a teaspoon the packet's size doesn't convert to.
    /// 3. Volumes and weights become one weight where the density is known: shops sell by weight.
    fn sum(&self, needs: &[Quantity], count: Option<&CountUnit>) -> Sum {
        let mut s = Sum {
            tsp: 0.0,
            grams: 0.0,
            counted: Vec::new(),
        };
        for q in needs {
            match q.unit.family() {
                Family::Volume => s.tsp += q.in_unit(&Unit::Tsp).unwrap_or(0.0),
                Family::Pinch => s.tsp += q.amount * PINCH_TSP,
                Family::Weight => s.grams += q.in_unit(&Unit::G).unwrap_or(0.0),
                Family::Count => {
                    let Unit::Count(noun) = &q.unit else { continue };
                    let noun = if noun.is_empty() {
                        count.map_or("", |c| c.noun.as_str())
                    } else {
                        noun
                    };
                    match s.counted.iter_mut().find(|(n, _)| n == noun) {
                        Some((_, a)) => *a += q.amount,
                        None => s.counted.push((noun.to_string(), q.amount)),
                    }
                }
            }
        }
        if s.parts() < 2 {
            return s;
        }
        let m = self.with_count(count);

        // 1. All in one count, where every amount converts to it: the food's own count word first, then the words
        // the recipes counted in, in their order.
        let words: Vec<String> = count
            .map(|c| c.noun.clone())
            .into_iter()
            .chain(s.counted.iter().map(|(n, _)| n.clone()))
            .filter(|n| !n.is_empty() && s.counted.iter().any(|(c, _)| c == n))
            .collect();
        for noun in words {
            let from_counts: Option<f64> = s
                .counted
                .iter()
                .map(|(n, a)| m.items_in(*a, n, &noun))
                .sum();
            let from_tsp = if s.tsp > 0.0 {
                m.item_tsp(&noun).map(|t| s.tsp / t)
            } else {
                Some(0.0)
            };
            let from_grams = if s.grams > 0.0 {
                m.item_grams(&noun).map(|g| s.grams / g)
            } else {
                Some(0.0)
            };
            if let (Some(c), Some(t), Some(g)) = (from_counts, from_tsp, from_grams) {
                return Sum {
                    tsp: 0.0,
                    grams: 0.0,
                    counted: vec![(noun, c + t + g)],
                };
            }
        }

        // 2. Counted items into the measure there is of them.
        let mut kept = Vec::new();
        for (noun, n) in std::mem::take(&mut s.counted) {
            if let Some(g) = m.item_grams(&noun).filter(|_| s.grams > 0.0) {
                s.grams += n * g;
            } else if let Some(t) = m.item_tsp(&noun).filter(|_| s.tsp > 0.0) {
                s.tsp += n * t;
            } else {
                kept.push((noun, n));
            }
        }
        s.counted = kept;

        // 3. One weight, where the density converts the volume.
        if let Some(d) = m.grams_per_ml.filter(|_| s.tsp > 0.0 && s.grams > 0.0) {
            s.grams += s.tsp / tsp_per_ml() * d;
            s.tsp = 0.0;
        }
        s
    }

    /// The total of `needs` in words, exactly: volumes and weights added up (weights in `weights`), counts as
    /// they come, converted where these measures say how (see the steps on `sum`). What doesn't convert is
    /// joined with "+": "1 TBSP + 2 cloves".
    ///
    /// ```
    /// use mise_core::quantity::{Quantity, Unit};
    /// use mise_core::shopping::{Measures, WeightUnit};
    ///
    /// let needs = [Quantity::new(1.0, Unit::Tbsp), Quantity::new(2.0, Unit::Count("clove".into()))];
    /// assert_eq!(Measures::default().total(&needs, None, WeightUnit::LbOz).unwrap(), "1 TBSP + 2 cloves");
    ///
    /// let mut garlic = Measures::default();
    /// garlic.add_item("clove", &Quantity::new(1.0, Unit::Tsp));
    /// assert_eq!(garlic.total(&needs, None, WeightUnit::LbOz).unwrap(), "5 cloves");
    /// ```
    pub fn total(
        &self,
        needs: &[Quantity],
        count: Option<&CountUnit>,
        weights: WeightUnit,
    ) -> Option<String> {
        let s = self.sum(needs, count);
        let mut parts = Vec::new();
        if s.grams > 0.0 {
            parts.push(weights.show(s.grams));
        }
        if s.tsp > 0.0 {
            parts.push(volume(s.tsp).exact());
        }
        for (noun, n) in s.counted {
            let q = Quantity::new(n, Unit::Count(noun.clone()));
            parts.push(
                match count
                    .filter(|c| c.noun == noun)
                    .and_then(|c| c.size.as_ref())
                {
                    Some(size) if size.unit.family() == Family::Weight => {
                        format!(
                            "{} ({})",
                            q.exact(),
                            weights
                                .for_size()
                                .show(size.scaled(n).in_unit(&Unit::G).unwrap_or(0.0))
                        )
                    }
                    Some(size) => format!("{} ({})", q.exact(), size.scaled(n).exact()),
                    None => q.exact(),
                },
            );
        }
        (!parts.is_empty()).then(|| parts.join(" + "))
    }

    /// Some of the total reads as a weight, so the household can choose how it reads.
    pub fn weighed(&self, needs: &[Quantity], count: Option<&CountUnit>) -> bool {
        let s = self.sum(needs, count);
        s.grams > 0.0
            || s.counted.iter().any(|(noun, _)| {
                count
                    .filter(|c| c.noun == *noun)
                    .and_then(|c| c.size.as_ref())
                    .is_some_and(|size| size.unit.family() == Family::Weight)
            })
    }
}

/// A size as curation writes it, "1 cup" or "8 oz": a volume or a weight, never a count.
pub fn parse_size(text: &str) -> Option<Quantity> {
    let text = text.trim();
    let (amount, used) = parse_number(text)?;
    let unit = Unit::parse(&text[used..])?;
    matches!(unit.family(), Family::Volume | Family::Weight)
        .then(|| Quantity::new(amount, unit))
        .filter(|q| q.amount > 0.0)
}

/// A food as the shopping list sees the food tree, to gather kinds of one food onto one line.
#[derive(Debug, Clone, Default)]
pub struct TreeFood {
    /// What it's a kind of.
    pub parent: Option<Uuid>,
    /// A category ("Vegetables") is never bought as such, so kinds never share a line through it.
    pub category: bool,
    /// Its kinds are bought as one (`shop_as_one` in the food's properties): two kinds of it share a line even
    /// when no recipe asks for the food itself.
    pub shop_as_one: bool,
}

/// The food whose line `food` goes on: itself, or the parent its kinds add up at.
///
/// A kind climbs to its parent only when a shopper could buy the two as one: the parent is a food, not a
/// category, and either a recipe in the plan asks for the parent itself (any onion does for "onion", so the
/// yellow onions another recipe wants cover it too), or curation says its kinds are bought as one
/// (`shop_as_one`). Otherwise a kind keeps its own line: a recipe that asks for red onion wants red, and nothing
/// ever climbs into a broad category such as Vegetables. It climbs on from there while the same holds.
pub fn line_food(food: Uuid, used: &HashSet<Uuid>, tree: &HashMap<Uuid, TreeFood>) -> Uuid {
    let mut at = food;
    let mut seen = HashSet::from([food]);
    while let Some(parent) = tree.get(&at).and_then(|f| f.parent) {
        let Some(p) = tree.get(&parent) else { break };
        if p.category || !(used.contains(&parent) || p.shop_as_one) || !seen.insert(parent) {
            break;
        }
        at = parent;
    }
    at
}

/// `food` and what it's a kind of, nearest first: Yellow Onion, Onion, Vegetables. A food without a section,
/// count unit or measures of its own takes its nearest ancestor's.
pub fn ancestors(food: Uuid, tree: &HashMap<Uuid, TreeFood>) -> Vec<Uuid> {
    let mut out = vec![food];
    while let Some(parent) = tree.get(out.last().unwrap_or(&food)).and_then(|f| f.parent) {
        if out.contains(&parent) {
            break;
        }
        out.push(parent);
    }
    out
}

/// Whether `food` is a kind of `ancestor`, at any depth.
pub fn is_kind_of(food: Uuid, ancestor: Uuid, tree: &HashMap<Uuid, TreeFood>) -> bool {
    food != ancestor && ancestors(food, tree).contains(&ancestor)
}

/// The foods that share one line, in the order given, and the key and name the line takes from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gathered {
    /// The line's food: the parent they add up at, or the one food.
    pub line: Uuid,
    /// The foods on it.
    pub foods: Vec<Uuid>,
    /// The food it is named after: the one food; the one kind, when the rest is the parent it covers (a recipe
    /// asking for onion takes the yellow onion another asks for); else the parent.
    pub named: Uuid,
}

/// The plan's foods (`used`, in the order the list should keep) gathered into lines by [`line_food`].
pub fn gather(used: &[Uuid], tree: &HashMap<Uuid, TreeFood>) -> Vec<Gathered> {
    let set: HashSet<Uuid> = used.iter().copied().collect();
    let mut lines: BTreeMap<usize, Gathered> = BTreeMap::new();
    let mut at: HashMap<Uuid, usize> = HashMap::new();
    for (i, f) in used.iter().enumerate() {
        let line = line_food(*f, &set, tree);
        let n = *at.entry(line).or_insert(i);
        let g = lines.entry(n).or_insert_with(|| Gathered {
            line,
            foods: Vec::new(),
            named: line,
        });
        if !g.foods.contains(f) {
            g.foods.push(*f);
        }
    }
    for g in lines.values_mut() {
        let kinds: Vec<Uuid> = g.foods.iter().copied().filter(|f| *f != g.line).collect();
        g.named = match (g.foods.as_slice(), kinds.as_slice()) {
            ([one], _) => *one,
            (_, [kind]) => *kind,
            _ => g.line,
        };
    }
    lines.into_values().collect()
}

/// A line on the list now, for working out what a tick sent for it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkLine {
    /// The key the list gives it.
    pub key: String,
    /// The keys of the foods on it: each food's id, or the name of an unmatched line.
    pub foods: Vec<String>,
}

/// The foods a tick (or a removal) sent with `key` marks. Ticks are stored per food, so a line that merges or
/// splits keeps them; a key names a line as the device last saw it, which may since have merged or split:
///
/// 1. A line that has the key, or a food named by it: every food on that line. Ticking a merged line ticks them
///    all, and a tick sent for a food's own line that has since merged ticks the merged line.
/// 2. Else lines with kinds of the food the key names: every food on them. A tick sent for a merged line that
///    has since split ticks each part.
/// 3. Else the key alone, for a food not on the list now.
pub fn foods_to_mark(
    key: &str,
    lines: &[MarkLine],
    kind_of: impl Fn(&str, &str) -> bool,
) -> Vec<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    if let Some(l) = lines
        .iter()
        .find(|l| l.key == key || l.foods.iter().any(|f| f == key))
    {
        out.extend(l.foods.iter().cloned());
    } else {
        for l in lines
            .iter()
            .filter(|l| l.foods.iter().any(|f| kind_of(f, key)))
        {
            out.extend(l.foods.iter().cloned());
        }
    }
    if out.is_empty() {
        out.insert(key.to_string());
    }
    out.into_iter().collect()
}

/// Whether a line shows as ticked (or removed): only when every food on it is. A merged line where some foods
/// were ticked before the merge and some weren't shows as not ticked, so nothing is left unbought.
pub fn all_marked(foods: &[String], marked: impl Fn(&str) -> bool) -> bool {
    !foods.is_empty() && foods.iter().all(|f| marked(f))
}

/// A volume in the largest unit it is a kitchen amount of (5 TBSP, not 0.31 cup), else the largest it reaches.
fn volume(tsp: f64) -> Quantity {
    let units: Vec<Quantity> = [(12.0, Unit::Cup), (3.0, Unit::Tbsp), (0.0, Unit::Tsp)]
        .into_iter()
        .filter(|(from, _)| tsp >= *from)
        .map(|(_, u)| Quantity::new(Quantity::new(tsp, Unit::Tsp).in_unit(&u).unwrap_or(tsp), u))
        .collect();
    units
        .iter()
        .find(|q| !format_exact(q.amount).contains('.'))
        .unwrap_or(&units[0])
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(a: f64, u: Unit) -> Quantity {
        Quantity::new(a, u)
    }
    fn bare(a: f64) -> Quantity {
        q(a, Unit::Count(String::new()))
    }

    const W: WeightUnit = WeightUnit::LbOz;

    #[test]
    fn adds_up_exactly_within_a_kind_of_measure() {
        assert_eq!(
            total(&[q(1.0, Unit::Tsp), q(2.0, Unit::Tsp)], None, W).unwrap(),
            "1 TBSP"
        );
        assert_eq!(
            total(&[q(2.0, Unit::Tbsp), q(2.0, Unit::Tbsp)], None, W).unwrap(),
            "¼ cup"
        );
        assert_eq!(
            total(&[q(1.0, Unit::Cup), q(0.3, Unit::Cup)], None, W).unwrap(),
            "1.3 cups"
        );
        assert_eq!(
            total(&[q(3.0, Unit::Tbsp), q(2.0, Unit::Tbsp)], None, W).unwrap(),
            "5 TBSP"
        );
        assert_eq!(
            total(&[q(1.0, Unit::Pinch), q(1.0, Unit::Pinch)], None, W).unwrap(),
            "⅛ tsp"
        );
        assert_eq!(
            total(&[q(8.0, Unit::Oz), q(4.0, Unit::Oz)], None, W).unwrap(),
            "12 oz"
        );
    }

    #[test]
    fn weights_read_in_the_households_units() {
        let beef = [q(10.0, Unit::Oz), q(10.0, Unit::Oz), q(10.0, Unit::Oz)];
        assert_eq!(total(&beef, None, WeightUnit::LbOz).unwrap(), "1 lb 14 oz");
        assert_eq!(total(&beef, None, WeightUnit::Oz).unwrap(), "30 oz");
        assert_eq!(total(&beef, None, WeightUnit::Lb).unwrap(), "1⅞ lb");
        assert_eq!(total(&beef, None, WeightUnit::Metric).unwrap(), "850 g");
        assert_eq!(
            total(
                &[q(16.0, Unit::Oz), q(16.0, Unit::Oz)],
                None,
                WeightUnit::LbOz
            )
            .unwrap(),
            "2 lb"
        );
        assert_eq!(
            total(
                &[q(1.0, Unit::Lb), q(0.3, Unit::Oz)],
                None,
                WeightUnit::LbOz
            )
            .unwrap(),
            "1 lb 0.3 oz"
        );
        assert_eq!(
            total(&[q(2.5, Unit::Lb)], None, WeightUnit::Metric).unwrap(),
            "1.13 kg"
        );
        assert_eq!(WeightUnit::parse("lb_oz"), Some(WeightUnit::LbOz));
        assert_eq!(WeightUnit::parse("stone"), None);
    }

    #[test]
    fn counts_stay_as_they_are() {
        let lemon = CountUnit::from_props(Some("whole"), None);
        assert_eq!(
            total(&[bare(0.5), bare(1.0)], lemon.as_ref(), W).unwrap(),
            "1½ whole"
        );
        assert_eq!(total(&[bare(0.3)], lemon.as_ref(), W).unwrap(), "0.3 whole");
        assert_eq!(total(&[bare(3.0)], None, W).unwrap(), "3");
    }

    #[test]
    fn counts_carry_their_size() {
        let paste = CountUnit::from_props(Some("packet"), Some("1.5 oz"));
        assert_eq!(
            total(&[bare(1.0), bare(2.0)], paste.as_ref(), W).unwrap(),
            "3 packets (4½ oz)"
        );
        assert_eq!(
            total(&[bare(1.0), bare(2.0)], paste.as_ref(), WeightUnit::Metric).unwrap(),
            "3 packets (128 g)"
        );
        assert_eq!(
            total(&[bare(1.0), bare(2.0)], paste.as_ref(), WeightUnit::Lb).unwrap(),
            "3 packets (4½ oz)"
        );
    }

    #[test]
    fn measures_of_a_counted_food_add_up_in_the_items_it_is_bought_in() {
        let paste = CountUnit::from_props(Some("packet"), Some("1.5 oz"));
        assert_eq!(
            total(&[bare(1.0), q(1.5, Unit::Oz)], paste.as_ref(), W).unwrap(),
            "2 packets (3 oz)"
        );
        let ketchup = CountUnit::from_props(Some("packet"), Some("2 TBSP"));
        assert_eq!(
            total(&[bare(1.0), q(1.0, Unit::Tbsp)], ketchup.as_ref(), W).unwrap(),
            "1½ packets (3 TBSP)"
        );
    }

    #[test]
    fn counted_items_fold_into_a_measure_when_not_everything_converts_to_them() {
        // A packet's weight says nothing about teaspoons, so the ounces can't become packets with the teaspoon.
        let paste = CountUnit::from_props(Some("packet"), Some("1.5 oz"));
        assert_eq!(
            total(
                &[bare(1.0), q(1.5, Unit::Oz), q(1.0, Unit::Tsp)],
                paste.as_ref(),
                W
            )
            .unwrap(),
            "3 oz + 1 tsp"
        );
    }

    fn garlic() -> Measures {
        let mut m = Measures::default();
        m.add_item("clove", &q(1.0, Unit::Tsp));
        m.add_item("head", &q(10.0, Unit::Tsp));
        m
    }

    fn clove(n: f64) -> Quantity {
        q(n, Unit::Count("clove".into()))
    }

    #[test]
    fn counts_and_measures_add_up_by_the_size_of_one_item() {
        assert_eq!(
            garlic()
                .total(&[clove(2.0), q(1.0, Unit::Tbsp)], None, W)
                .unwrap(),
            "5 cloves"
        );
        // Another item converts too, when both sizes are known in one kind of measure.
        let head = q(1.0, Unit::Count("head".into()));
        assert_eq!(
            garlic().total(&[clove(2.0), head], None, W).unwrap(),
            "12 cloves"
        );
        // An item sized by weight converts a volume through the density.
        let mut onion = Measures::default();
        onion.add_item("onion", &q(8.0, Unit::Oz));
        onion.add_density(236.6, 160.0);
        let onions = q(1.0, Unit::Count("onion".into()));
        assert_eq!(
            onion.total(&[onions, q(1.0, Unit::Cup)], None, W).unwrap(),
            "1.71 onions"
        );
    }

    #[test]
    fn one_kind_of_amount_reads_as_written() {
        assert_eq!(
            garlic().total(&[q(1.0, Unit::Tbsp)], None, W).unwrap(),
            "1 TBSP"
        );
        assert_eq!(garlic().total(&[clove(2.0)], None, W).unwrap(), "2 cloves");
    }

    #[test]
    fn volume_and_weight_add_up_as_one_weight_by_density() {
        let mut parmesan = Measures::default();
        parmesan.add_density(
            Quantity::new(1.0, Unit::Cup).in_unit(&Unit::Ml).unwrap(),
            85.05,
        );
        let needs = [q(0.5, Unit::Cup), q(1.5, Unit::Oz)];
        assert_eq!(parmesan.total(&needs, None, W).unwrap(), "3 oz");
        assert_eq!(
            parmesan.total(&needs, None, WeightUnit::Metric).unwrap(),
            "85 g"
        );
        assert!(parmesan.weighed(&needs, None));
        // Without a density the two stay apart.
        assert_eq!(
            Measures::default().total(&needs, None, W).unwrap(),
            "1½ oz + ½ cup"
        );
    }

    #[test]
    fn a_count_with_no_size_per_item_stays_apart() {
        // A density is no help: it says what a cup weighs, not how big a clove is.
        let mut no_size = Measures::default();
        no_size.add_density(1.0, 0.6);
        assert_eq!(
            no_size
                .total(&[clove(2.0), q(1.0, Unit::Tbsp)], None, W)
                .unwrap(),
            "1 TBSP + 2 cloves"
        );
        // Nor is another item's size.
        let mut heads_only = Measures::default();
        heads_only.add_item("head", &q(10.0, Unit::Tsp));
        assert_eq!(
            heads_only
                .total(&[clove(2.0), q(1.0, Unit::Tbsp)], None, W)
                .unwrap(),
            "1 TBSP + 2 cloves"
        );
    }

    #[test]
    fn a_foods_own_measures_come_before_its_parents() {
        let mut yellow = Measures::default();
        yellow.add_item("onion", &q(1.5, Unit::Cup));
        let mut onion = Measures::default();
        onion.add_item("onion", &q(1.0, Unit::Cup));
        onion.add_density(1.0, 0.68);
        yellow.add(&onion);
        assert_eq!(yellow.items.len(), 1);
        let onions = q(1.0, Unit::Count("onion".into()));
        assert_eq!(
            yellow
                .total(&[onions, q(0.75, Unit::Cup)], None, W)
                .unwrap(),
            "1½ onions"
        );
        assert_eq!(yellow.grams_per_ml, Some(0.68));
    }

    #[test]
    fn sizes_read_from_curation_are_volumes_or_weights() {
        assert_eq!(parse_size("1 cup"), Some(q(1.0, Unit::Cup)));
        assert_eq!(parse_size(" 1½ oz "), Some(q(1.5, Unit::Oz)));
        assert_eq!(parse_size("2 cloves"), None);
        assert_eq!(parse_size("a pinch"), None);
        assert_eq!(parse_size("0 g"), None);
    }

    /// A food tree: (name, parent, category, shop_as_one), with ids made from the names.
    fn tree(foods: &[(&str, Option<&str>, bool, bool)]) -> HashMap<Uuid, TreeFood> {
        foods
            .iter()
            .map(|(name, parent, category, shop_as_one)| {
                (
                    id(name),
                    TreeFood {
                        parent: parent.map(id),
                        category: *category,
                        shop_as_one: *shop_as_one,
                    },
                )
            })
            .collect()
    }

    /// An id that is the same for the same name, so tests can name foods.
    fn id(name: &str) -> Uuid {
        let mut bytes = [0u8; 16];
        for (i, b) in name.bytes().enumerate() {
            bytes[i % 16] = bytes[i % 16].wrapping_mul(31).wrapping_add(b);
        }
        bytes[15] = bytes[15].wrapping_add(name.len() as u8);
        Uuid::from_bytes(bytes)
    }

    fn onions() -> HashMap<Uuid, TreeFood> {
        tree(&[
            ("Vegetables", None, true, false),
            ("Onion", Some("Vegetables"), false, false),
            ("Yellow Onion", Some("Onion"), false, false),
            ("Red Onion", Some("Onion"), false, false),
            ("Carrot", Some("Vegetables"), false, false),
            ("Pepper", None, false, true),
            ("Bell Pepper", Some("Pepper"), false, false),
            ("Poblano", Some("Pepper"), false, false),
        ])
    }

    /// The plan's foods gathered into lines, each as the names of its foods.
    fn lines(used: &[&str], tree: &HashMap<Uuid, TreeFood>) -> Vec<(Vec<String>, String)> {
        let names: HashMap<Uuid, &str> = [
            "Vegetables",
            "Onion",
            "Yellow Onion",
            "Red Onion",
            "Carrot",
            "Pepper",
            "Bell Pepper",
            "Poblano",
        ]
        .into_iter()
        .map(|n| (id(n), n))
        .collect();
        let used: Vec<Uuid> = used.iter().map(|n| id(n)).collect();
        gather(&used, tree)
            .into_iter()
            .map(|g| {
                (
                    g.foods.iter().map(|f| names[f].to_string()).collect(),
                    names[&g.named].to_string(),
                )
            })
            .collect()
    }

    fn line(foods: &[&str], named: &str) -> (Vec<String>, String) {
        (
            foods.iter().map(|f| f.to_string()).collect(),
            named.to_string(),
        )
    }

    #[test]
    fn kinds_of_a_food_share_its_line_when_a_recipe_asks_for_the_food_itself() {
        let t = onions();
        assert_eq!(
            lines(&["Yellow Onion", "Onion", "Carrot"], &t),
            [
                line(&["Yellow Onion", "Onion"], "Yellow Onion"),
                line(&["Carrot"], "Carrot")
            ]
        );
        assert_eq!(
            lines(&["Onion", "Yellow Onion", "Red Onion"], &t),
            [line(&["Onion", "Yellow Onion", "Red Onion"], "Onion")]
        );
    }

    #[test]
    fn kinds_alone_keep_their_own_lines_unless_curation_buys_them_as_one() {
        let t = onions();
        // A recipe that asks for red onion wants red.
        assert_eq!(
            lines(&["Yellow Onion", "Red Onion"], &t),
            [
                line(&["Yellow Onion"], "Yellow Onion"),
                line(&["Red Onion"], "Red Onion")
            ]
        );
        // Peppers are marked as bought as one, so two kinds share a line named for the parent.
        assert_eq!(
            lines(&["Bell Pepper", "Poblano"], &t),
            [line(&["Bell Pepper", "Poblano"], "Pepper")]
        );
        // And one kind alone keeps its own name.
        assert_eq!(lines(&["Poblano"], &t), [line(&["Poblano"], "Poblano")]);
    }

    #[test]
    fn nothing_shares_a_line_through_a_category() {
        let t = onions();
        assert_eq!(
            lines(&["Onion", "Carrot", "Vegetables"], &t),
            [
                line(&["Onion"], "Onion"),
                line(&["Carrot"], "Carrot"),
                line(&["Vegetables"], "Vegetables")
            ]
        );
        assert!(is_kind_of(id("Yellow Onion"), id("Vegetables"), &t));
        assert!(!is_kind_of(id("Onion"), id("Onion"), &t));
        assert_eq!(
            ancestors(id("Yellow Onion"), &t),
            [id("Yellow Onion"), id("Onion"), id("Vegetables")]
        );
    }

    /// The list's lines for a tick to land on: (key, foods).
    fn on_list(lines: &[(&str, &[&str])]) -> Vec<MarkLine> {
        lines
            .iter()
            .map(|(key, foods)| MarkLine {
                key: key.to_string(),
                foods: foods.iter().map(|f| f.to_string()).collect(),
            })
            .collect()
    }

    /// "yellow onion" is a kind of "onion", and so on: kinds end in their parent's name.
    fn kind_of(food: &str, of: &str) -> bool {
        food != of && food.ends_with(&format!(" {of}"))
    }

    #[test]
    fn a_merged_line_is_ticked_only_when_every_line_it_replaced_was() {
        let foods = ["onion".to_string(), "yellow onion".to_string()];
        assert!(all_marked(&foods, |_| true));
        assert!(!all_marked(&foods, |f| f == "onion"), "a mix is not ticked");
        assert!(!all_marked(&foods, |_| false));
        assert!(!all_marked(&[], |_| true));
    }

    #[test]
    fn ticking_a_merged_line_ticks_every_food_on_it() {
        let list = on_list(&[("onion", &["yellow onion", "onion"]), ("lime", &["lime"])]);
        assert_eq!(
            foods_to_mark("onion", &list, kind_of),
            ["onion", "yellow onion"]
        );
        assert_eq!(foods_to_mark("lime", &list, kind_of), ["lime"]);
    }

    #[test]
    fn a_tick_queued_for_a_line_that_has_since_merged_ticks_the_merged_line() {
        // The device saw a Yellow Onion line; another recipe asking for onion has merged it since.
        let list = on_list(&[("onion", &["yellow onion", "onion"])]);
        assert_eq!(
            foods_to_mark("yellow onion", &list, kind_of),
            ["onion", "yellow onion"]
        );
    }

    #[test]
    fn a_tick_queued_for_a_line_that_has_since_split_ticks_each_part() {
        // The device saw one onion line; the recipe asking for onion has left the plan since.
        let list = on_list(&[
            ("yellow onion", &["yellow onion"]),
            ("red onion", &["red onion"]),
            ("lime", &["lime"]),
        ]);
        assert_eq!(
            foods_to_mark("onion", &list, kind_of),
            ["red onion", "yellow onion"]
        );
        // A food no longer on the list keeps its tick for when it's back.
        assert_eq!(foods_to_mark("carrot", &list, kind_of), ["carrot"]);
    }

    #[test]
    fn a_line_split_by_a_plan_change_keeps_each_parts_tick() {
        // Ticks are kept per food: Yellow Onion was ticked on the merged line, Red Onion and Onion weren't.
        let ticked = ["yellow onion"];
        let marked = |f: &str| ticked.contains(&f);
        let merged = ["onion", "yellow onion", "red onion"].map(String::from);
        assert!(!all_marked(&merged, marked));
        // The recipe asking for onion leaves the plan: each kind is its own line again, with its own tick.
        assert!(all_marked(&["yellow onion".to_string()], marked));
        assert!(!all_marked(&["red onion".to_string()], marked));
    }

    #[test]
    fn what_will_not_convert_is_joined() {
        let garlic = q(2.0, Unit::Count("clove".into()));
        assert_eq!(
            total(&[garlic, q(1.0, Unit::Tbsp)], None, W).unwrap(),
            "1 TBSP + 2 cloves"
        );
        let lettuce = CountUnit::from_props(Some("head"), None);
        assert_eq!(
            total(&[bare(1.0), q(4.0, Unit::Oz)], lettuce.as_ref(), W).unwrap(),
            "4 oz + 1 head"
        );
    }

    #[test]
    fn nothing_to_add_up_has_no_amount() {
        assert_eq!(total(&[], None, W), None);
    }

    #[test]
    fn sections_sort_as_the_shop_is_walked() {
        assert!(section_order("Produce") < section_order("Baking"));
        assert!(section_order("Baking") < section_order(OTHER));
        assert!(section_order(OTHER) < section_order(CHECK));
        assert_eq!(section_order("Somewhere new"), section_order(OTHER));
    }
}
