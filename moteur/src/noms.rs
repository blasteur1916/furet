//! Recherche par nom de fichier, comme Everything : sans accents ni majuscules, avec une petite tolérance aux fautes.

/// « Facture_ÉDF-2026.PDF » → « facture edf 2026 pdf » ; « MonFichier » → « mon fichier ».
pub fn normaliser(texte: &str) -> String {
    let mut sortie = String::with_capacity(texte.len());
    let mut precedent: Option<char> = None;
    for c in texte.chars() {
        let coupure = match precedent {
            Some(p) => (p.is_lowercase() && c.is_uppercase()) || (p.is_alphabetic() && c.is_ascii_digit())
                || (p.is_ascii_digit() && c.is_alphabetic()),
            None => false,
        };
        if coupure {
            sortie.push(' ');
        }
        for minuscule in c.to_lowercase() {
            match sans_accent(minuscule) {
                Some(s) => sortie.push_str(s),
                None if minuscule.is_alphanumeric() => sortie.push(minuscule),
                None => sortie.push(' '),
            }
        }
        precedent = Some(c);
    }
    sortie.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn sans_accent(c: char) -> Option<&'static str> {
    Some(match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' | 'å' => "a",
        'ç' => "c",
        'è' | 'é' | 'ê' | 'ë' => "e",
        'ì' | 'í' | 'î' | 'ï' => "i",
        'ñ' => "n",
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' => "o",
        'ù' | 'ú' | 'û' | 'ü' => "u",
        'ý' | 'ÿ' => "y",
        'œ' => "oe",
        'æ' => "ae",
        'ß' => "ss",
        _ => return None,
    })
}

/// Score de 0 à 1 d'un nom (normalisé) pour une recherche (normalisée). 0 : ne correspond pas.
pub fn score(recherche: &str, nom: &str) -> f32 {
    score_avec(recherche, nom, true)
}

/// Tous les noms, en mémoire : la recherche par nom doit répondre à chaque touche.
pub struct IndexNoms {
    /// (nom normalisé, chemin)
    entrees: Vec<(String, String)>,
}

impl IndexNoms {
    pub fn nouveau(entrees: Vec<(String, String)>) -> Self {
        IndexNoms { entrees }
    }

    pub fn len(&self) -> usize {
        self.entrees.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entrees.is_empty()
    }

    /// Les `n` meilleurs (score, chemin). D'abord sans fautes de frappe (rapide) ; la tolérance
    /// aux fautes, plus coûteuse, ne sert que s'il manque des résultats.
    pub fn chercher(&self, recherche: &str, n: usize, garder: impl Fn(&str) -> bool) -> Vec<(f32, &str)> {
        let recherche = normaliser(recherche);
        if recherche.is_empty() || n == 0 {
            return Vec::new();
        }
        let passe = |fautes: bool| -> Vec<(f32, &str)> {
            self.entrees
                .iter()
                .filter_map(|(cle, chemin)| {
                    let s = score_avec(&recherche, cle, fautes);
                    (s > 0.0 && garder(chemin)).then_some((s, chemin.as_str()))
                })
                .collect()
        };
        let mut trouves = passe(false);
        if trouves.len() < n {
            trouves = passe(true);
        }
        trouves.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
        trouves.truncate(n);
        trouves
    }
}

pub fn score_avec(recherche: &str, nom: &str, fautes: bool) -> f32 {
    if recherche.is_empty() {
        return 0.0;
    }
    if nom.starts_with(recherche) {
        return 1.0;
    }
    if nom.contains(recherche) {
        return 0.95;
    }
    // Sinon, chaque mot cherché doit se retrouver dans un mot du nom.
    let mots_nom: Vec<&str> = nom.split(' ').collect();
    let mots: Vec<&str> = recherche.split(' ').collect();
    let mut total = 0.0;
    for m in &mots {
        let meilleur = mots_nom.iter().map(|t| score_mot(m, t, fautes)).fold(0.0, f32::max);
        if meilleur == 0.0 {
            return 0.0;
        }
        total += meilleur;
    }
    0.9 * total / mots.len() as f32
}

fn score_mot(m: &str, t: &str, fautes: bool) -> f32 {
    if t == m {
        return 1.0;
    }
    if t.starts_with(m) {
        return 0.95;
    }
    if m.chars().count() >= 3 && t.contains(m) {
        return 0.85;
    }
    if !fautes {
        return 0.0;
    }
    // Petite faute de frappe : 1 lettre à partir de 4, 2 à partir de 8.
    let n = m.chars().count();
    let tolerance = match n {
        0..=3 => return 0.0,
        4..=7 => 1,
        _ => 2,
    };
    let debut_t: String = t.chars().take(n).collect();
    let ecart = distance(m, t).min(distance(m, &debut_t) + usize::from(t.chars().count() > n));
    if ecart <= tolerance { 0.75 - 0.1 * (ecart as f32 - 1.0) } else { 0.0 }
}

/// Distance de Levenshtein (insertions, suppressions, remplacements).
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut ligne: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonale = ligne[0];
        ligne[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let ancien = ligne[j + 1];
            ligne[j + 1] = (ligne[j + 1] + 1).min(ligne[j] + 1).min(diagonale + usize::from(ca != *cb));
            diagonale = ancien;
        }
    }
    ligne[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalise() {
        assert_eq!(normaliser("Facture_ÉDF-2026.PDF"), "facture edf 2026 pdf");
        assert_eq!(normaliser("MonFichierPréféré.docx"), "mon fichier prefere docx");
        assert_eq!(normaliser("IMG2045.jpg"), "img 2045 jpg");
    }

    #[test]
    fn trouve() {
        let nom = normaliser("suspicious.gif");
        assert_eq!(score(&normaliser("suspicious"), &nom), 1.0);
        assert!(score(&normaliser("suspicious.gif"), &nom) > 0.9);
        assert!(score(&normaliser("gif"), &nom) > 0.8);
        assert!(score(&normaliser("suspicous"), &nom) > 0.5, "une lettre oubliée");
        assert!(score(&normaliser("suspi"), &nom) > 0.9, "début du nom");
        assert_eq!(score(&normaliser("facture"), &nom), 0.0);
        let releve = normaliser("Relevé bancaire mars.pdf");
        assert!(score(&normaliser("releve mars"), &releve) > 0.8, "plusieurs mots, sans accents");
        assert!(score(&normaliser("bancaire releve"), &releve) > 0.8, "dans le désordre");
    }
}
