//! Mise en forme de listes dans du texte brut (puces, numéros, lettres, romains, cases à cocher).

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ListKind {
    Bullet,
    Dash,
    Numbered,
    LetterLower,
    LetterUpper,
    RomanLower,
    RomanUpper,
    Checkbox,
}

impl ListKind {
    pub const ALL: [(ListKind, &'static str); 8] = [
        (ListKind::Bullet, "• Liste à puces"),
        (ListKind::Dash, "- Liste à tirets"),
        (ListKind::Numbered, "1. Liste numérotée"),
        (ListKind::LetterLower, "a. Liste alphabétique (minuscules)"),
        (ListKind::LetterUpper, "A. Liste alphabétique (majuscules)"),
        (ListKind::RomanLower, "i. Liste romaine (minuscules)"),
        (ListKind::RomanUpper, "I. Liste romaine (majuscules)"),
        (ListKind::Checkbox, "[ ] Liste de tâches"),
    ];

    /// Marqueur (sans espace final) pour le n-ième élément (à partir de 1).
    pub fn marker(self, n: usize) -> String {
        match self {
            ListKind::Bullet => "•".into(),
            ListKind::Dash => "-".into(),
            ListKind::Numbered => format!("{n}."),
            ListKind::LetterLower => format!("{}.", letters(n, false)),
            ListKind::LetterUpper => format!("{}.", letters(n, true)),
            ListKind::RomanLower => format!("{}.", roman(n).to_lowercase()),
            ListKind::RomanUpper => format!("{}.", roman(n)),
            ListKind::Checkbox => "[ ]".into(),
        }
    }
}

fn letters(mut n: usize, upper: bool) -> String {
    let mut out = Vec::new();
    while n > 0 {
        n -= 1;
        let c = (b'a' + (n % 26) as u8) as char;
        out.push(if upper { c.to_ascii_uppercase() } else { c });
        n /= 26;
    }
    out.iter().rev().collect()
}

fn from_letters(s: &str) -> Option<usize> {
    if s.is_empty() || s.len() > 3 {
        return None;
    }
    let mut n = 0usize;
    for c in s.chars() {
        let c = c.to_ascii_lowercase();
        if !c.is_ascii_lowercase() {
            return None;
        }
        n = n * 26 + (c as u8 - b'a') as usize + 1;
    }
    Some(n)
}

fn roman(mut n: usize) -> String {
    const T: [(usize, &str); 13] = [
        (1000, "M"), (900, "CM"), (500, "D"), (400, "CD"), (100, "C"), (90, "XC"),
        (50, "L"), (40, "XL"), (10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I"),
    ];
    let mut s = String::new();
    for (v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

fn from_roman(s: &str) -> Option<usize> {
    if s.is_empty() || s.len() > 8 {
        return None;
    }
    let val = |c: char| match c.to_ascii_uppercase() {
        'I' => Some(1), 'V' => Some(5), 'X' => Some(10), 'L' => Some(50),
        'C' => Some(100), 'D' => Some(500), 'M' => Some(1000), _ => None,
    };
    let v: Option<Vec<i64>> = s.chars().map(val).collect();
    let v = v?;
    let mut total = 0i64;
    for i in 0..v.len() {
        if i + 1 < v.len() && v[i] < v[i + 1] {
            total -= v[i];
        } else {
            total += v[i];
        }
    }
    let total = usize::try_from(total).ok()?;
    (roman(total).eq_ignore_ascii_case(s)).then_some(total)
}

/// Résultat de l'analyse d'une ligne.
#[derive(Debug, Clone)]
pub struct Item {
    pub indent: String,
    pub kind: ListKind,
    pub number: usize,
    /// Longueur en octets de « indentation + marqueur + espace ».
    pub prefix_len: usize,
    /// La ligne ne contient que le marqueur.
    pub empty: bool,
}

pub fn parse(line: &str) -> Option<Item> {
    let body = line.trim_start_matches([' ', '\t']);
    let indent = &line[..line.len() - body.len()];
    let (word, rest) = match body.find(' ') {
        Some(i) => (&body[..i], &body[i + 1..]),
        None => (body, ""),
    };
    // Une ligne « marqueur seul » (sans espace) est aussi une puce vide.
    let has_space = body.contains(' ');
    let kind_num: Option<(ListKind, usize)> = match word {
        "•" => Some((ListKind::Bullet, 1)),
        "-" | "*" => Some((ListKind::Dash, 1)),
        "[ ]" | "[x]" | "[X]" => Some((ListKind::Checkbox, 1)),
        _ => word.strip_suffix('.').and_then(|w| {
            if w.is_empty() {
                None
            } else if w.chars().all(|c| c.is_ascii_digit()) {
                w.parse().ok().map(|n| (ListKind::Numbered, n))
            } else if w.chars().all(|c| c.is_ascii_lowercase()) {
                // Les chiffres romains minuscules ont priorité pour i, v, x…
                if let Some(n) = from_roman(w).filter(|_| w.len() > 1 || "ivx".contains(w)) {
                    Some((ListKind::RomanLower, n))
                } else {
                    from_letters(w).map(|n| (ListKind::LetterLower, n))
                }
            } else if w.chars().all(|c| c.is_ascii_uppercase()) {
                if let Some(n) = from_roman(w).filter(|_| w.len() > 1 || "IVX".contains(w)) {
                    Some((ListKind::RomanUpper, n))
                } else {
                    from_letters(w).map(|n| (ListKind::LetterUpper, n))
                }
            } else {
                None
            }
        }),
    };
    let (kind, number) = kind_num?;
    if !has_space && !rest.is_empty() {
        return None;
    }
    // Un simple « - » ou « * » collé à du texte n'est pas une liste (géré par has_space).
    if !has_space && body.len() != word.len() {
        return None;
    }
    let prefix_len = indent.len() + word.len() + usize::from(has_space);
    Some(Item {
        indent: indent.to_string(),
        kind,
        number,
        prefix_len,
        empty: rest.trim().is_empty(),
    })
}

/// Marqueur suivant pour continuer une liste (touche Entrée).
pub fn next_marker(item: &Item) -> String {
    format!("{}{} ", item.indent, item.kind.marker(item.number + 1))
}

fn strip(line: &str) -> String {
    match parse(line) {
        Some(it) => format!("{}{}", it.indent, &line[it.prefix_len..]),
        None => line.to_string(),
    }
}

/// Applique (ou retire, si toutes les lignes sont déjà de ce type) une liste sur `lines`.
pub fn apply(lines: &[String], kind: ListKind) -> Vec<String> {
    let non_empty = |l: &&String| !l.trim().is_empty();
    let all_same = lines.iter().filter(non_empty).count() > 0
        && lines
            .iter()
            .filter(non_empty)
            .all(|l| parse(l).is_some_and(|it| it.kind == kind));
    let mut n = 0;
    lines
        .iter()
        .map(|l| {
            if l.trim().is_empty() {
                return l.clone();
            }
            let base = strip(l);
            if all_same {
                return base;
            }
            n += 1;
            let body = base.trim_start_matches([' ', '\t']);
            let indent = &base[..base.len() - body.len()];
            format!("{indent}{} {body}", kind.marker(n))
        })
        .collect()
}

pub fn remove(lines: &[String]) -> Vec<String> {
    lines.iter().map(|l| strip(l)).collect()
}

pub const INDENT: &str = "    ";

pub fn indent(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| if l.is_empty() { l.clone() } else { format!("{INDENT}{l}") })
        .collect()
}

pub fn outdent(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| {
            if let Some(r) = l.strip_prefix(INDENT) {
                r.to_string()
            } else if let Some(r) = l.strip_prefix('\t') {
                r.to_string()
            } else {
                l.trim_start_matches(' ').to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numerotation() {
        let l: Vec<String> = vec!["a".into(), "b".into(), "".into(), "c".into()];
        let r = apply(&l, ListKind::Numbered);
        assert_eq!(r, vec!["1. a", "2. b", "", "3. c"]);
        assert_eq!(apply(&r, ListKind::Numbered), l); // bascule : retire la liste
        assert_eq!(apply(&r, ListKind::Bullet), vec!["• a", "• b", "", "• c"]);
    }

    #[test]
    fn romain_lettres() {
        assert_eq!(roman(14), "XIV");
        assert_eq!(letters(27, false), "aa");
        assert_eq!(parse("iv. test").unwrap().number, 4);
        assert_eq!(parse("b. test").unwrap().kind, ListKind::LetterLower);
        assert_eq!(parse("    - x").unwrap().indent, "    ");
        assert!(parse("-x").is_none());
        assert!(parse("texte normal").is_none());
    }

    #[test]
    fn suite() {
        let it = parse("  3. truc").unwrap();
        assert_eq!(next_marker(&it), "  4. ");
        assert!(parse("2. ").unwrap().empty);
    }
}
