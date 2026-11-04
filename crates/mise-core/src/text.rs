//! Text normalization used for matching names across sources.

/// Lowercase, fold "®"/"™"/curly quotes, keep letters, digits and single spaces.
/// Used for food aliases and dish-title comparison.
pub fn normalize_name(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for ch in s.chars() {
        let ch = match ch {
            '’' | '‘' => '\'',
            '®' | '™' | '©' => ' ',
            c => c,
        };
        if ch.is_alphanumeric() || ch == '\'' {
            out.extend(ch.to_lowercase());
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim_end().to_string()
}

/// URL-safe slug: normalized name with spaces as hyphens, apostrophes dropped.
pub fn slugify(s: &str) -> String {
    normalize_name(s).replace('\'', "").replace(' ', "-")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_brands_and_punctuation() {
        assert_eq!(
            normalize_name("PHILADELPHIA® Cream Cheese"),
            "philadelphia cream cheese"
        );
        assert_eq!(
            normalize_name("  Frank’s RedHot®  Sauce "),
            "frank's redhot sauce"
        );
        assert_eq!(normalize_name("Tex-Mex Paste"), "tex mex paste");
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Frank’s RedHot® Sauce"), "franks-redhot-sauce");
        assert_eq!(
            slugify("Sweet & Smoky BBQ Seasoning"),
            "sweet-smoky-bbq-seasoning"
        );
    }
}
