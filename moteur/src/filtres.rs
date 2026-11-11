//! Les filtres tapés dans la barre, retirés du texte cherché : le type (« photos », « pdf », « vidéos »…),
//! la date de modification (« 2024 », « mars 2023 », « hier », « la semaine dernière », « avant 2020 »…) et
//! la taille (« > 1 Go », « moins de 5 Mo »). Portable : rien ici n'est propre à Windows.

use std::path::Path;
use std::time::UNIX_EPOCH;

use chrono::{Datelike, Days, Local, Months, NaiveDate, TimeZone};

use crate::fichiers::Genre;
use crate::noms::normaliser;

/// Ce que les fichiers trouvés doivent respecter.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filtre {
    /// Types gardés : noms de `Genre`, ou « dossier ». Vide : tous.
    pub genres: Vec<&'static str>,
    /// Modifiés à partir de ce moment (secondes depuis 1970)…
    pub depuis: Option<i64>,
    /// … et avant celui-ci.
    pub avant: Option<i64>,
    /// En octets.
    pub taille_min: Option<u64>,
    pub taille_max: Option<u64>,
    /// Seulement ces fichiers (par exemple les photos prises à un lieu).
    pub parmi: Option<std::collections::HashSet<String>>,
}

impl Filtre {
    pub fn est_vide(&self) -> bool {
        *self == Filtre::default()
    }

    pub fn garde_parmi(&self, chemin: &str) -> bool {
        self.parmi.as_ref().is_none_or(|p| p.contains(chemin))
    }

    pub fn garde_genre(&self, genre: &str) -> bool {
        self.genres.is_empty() || self.genres.contains(&genre)
    }

    /// Faut-il connaître la taille ou la date des fichiers ?
    fn par_infos(&self) -> bool {
        self.depuis.is_some() || self.avant.is_some() || self.taille_min.is_some() || self.taille_max.is_some()
    }

    pub fn garde_infos(&self, taille: u64, modifie: i64) -> bool {
        self.depuis.is_none_or(|d| modifie >= d)
            && self.avant.is_none_or(|a| modifie < a)
            && self.taille_min.is_none_or(|t| taille >= t)
            && self.taille_max.is_none_or(|t| taille <= t)
    }

    /// Pour un chemin trouvé par son nom : le type d'après l'extension ; la taille et la date en lisant le
    /// disque, seulement si un filtre les demande.
    pub fn garde_chemin(&self, chemin: &Path, dossier: bool) -> bool {
        let genre = if dossier { "dossier" } else { Genre::de(chemin).map_or("fichier", |g| g.nom()) };
        if !self.garde_genre(genre) {
            return false;
        }
        if !self.par_infos() {
            return true;
        }
        // La taille d'un dossier n'est pas connue.
        if dossier && (self.taille_min.is_some() || self.taille_max.is_some()) {
            return false;
        }
        let Ok(infos) = std::fs::metadata(chemin) else { return false };
        let modifie = infos.modified().ok().and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
        self.garde_infos(infos.len(), modifie)
    }
}

/// Une recherche, séparée en texte et filtres.
#[derive(Debug, Clone, Default)]
pub struct Analyse {
    /// Ce qui reste à chercher (le texte d'origine s'il n'y a aucun filtre).
    pub reste: String,
    pub filtre: Filtre,
    /// Les filtres compris, à montrer : « Photos », « 2024 », « > 1 Go ».
    pub etiquettes: Vec<String>,
}

/// (mots, types gardés, étiquette)
const TYPES: [(&[&str], &[&str], &str); 9] = [
    (&["photo", "photos", "image", "images"], &["photo"], "Photos"),
    (&["pdf", "pdfs"], &["pdf"], "PDF"),
    (&["video", "videos", "film", "films"], &["video"], "Vidéos"),
    (&["audio", "audios", "musique", "musiques", "chanson", "chansons", "enregistrement", "enregistrements"], &["audio"], "Audio"),
    (&["document", "documents"], &["document", "pdf"], "Documents"),
    (&["tableur", "tableurs", "excel"], &["tableur"], "Tableurs"),
    (&["presentation", "presentations", "diaporama", "diaporamas", "powerpoint"], &["document"], "Présentations"),
    (&["dossier", "dossiers"], &["dossier"], "Dossiers"),
    (&["archive", "archives", "zip", "zips"], &["archive"], "Archives"),
];

const MOIS: [&str; 12] =
    ["janvier", "fevrier", "mars", "avril", "mai", "juin", "juillet", "aout", "septembre", "octobre", "novembre", "decembre"];
const MOIS_LISIBLES: [&str; 12] =
    ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];

/// Retirés au début et à la fin de ce qui reste : « photos de chat » → « chat ».
const LIAISONS: [&str; 15] =
    ["de", "du", "des", "d", "en", "le", "la", "les", "l", "a", "et", "fichier", "fichiers", "mes", "modifie"];
/// « modifié la semaine dernière » : le mot ne change rien, il est retiré.
const MODIFIE: [&str; 8] = ["modifie", "modifies", "modifiee", "modifiees", "cree", "crees", "creee", "creees"];

/// Les mots de la recherche : les opérateurs à part, « l'année » → « l' » + « année », « 1Go » → « 1 » + « Go ».
fn decouper(texte: &str) -> Vec<String> {
    let mut mots = Vec::new();
    let mut courant = String::new();
    let pousser = |courant: &mut String, mots: &mut Vec<String>| {
        if !courant.is_empty() {
            mots.push(std::mem::take(courant));
        }
    };
    for c in texte.chars() {
        let nombre = !courant.is_empty() && courant.chars().all(|d| d.is_ascii_digit() || d == ',' || d == '.');
        if c.is_whitespace() {
            pousser(&mut courant, &mut mots);
        } else if matches!(c, '>' | '<' | '≥' | '≤') {
            pousser(&mut courant, &mut mots);
            mots.push(c.to_string());
        } else if c == '\'' || c == '’' {
            courant.push(c);
            pousser(&mut courant, &mut mots);
        } else if c.is_alphabetic() && nombre {
            pousser(&mut courant, &mut mots);
            courant.push(c);
        } else {
            courant.push(c);
        }
    }
    pousser(&mut courant, &mut mots);
    mots
}

fn debut_du_jour(jour: NaiveDate) -> i64 {
    let minuit = jour.and_hms_opt(0, 0, 0).unwrap_or_default();
    Local.from_local_datetime(&minuit).earliest().map_or_else(|| minuit.and_utc().timestamp(), |t| t.timestamp())
}

fn premier_du_mois(annee: i32, mois: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(annee, mois, 1).unwrap_or_default()
}

/// Une période [début, fin[ en jours.
struct Periode {
    debut: Option<NaiveDate>,
    fin: Option<NaiveDate>,
    etiquette: String,
}

/// Une année plausible pour une date de fichier.
fn annee(mot: &str, aujourdhui: NaiveDate) -> Option<i32> {
    let a: i32 = mot.parse().ok().filter(|_| mot.len() == 4)?;
    (1970..=aujourdhui.year() + 1).contains(&a).then_some(a)
}

fn mois(mot: &str) -> Option<u32> {
    MOIS.iter().position(|m| *m == mot).map(|i| i as u32 + 1)
}

/// Une date à la position `i` : (mots pris, période).
fn date(s: &[String], i: usize, aujourdhui: NaiveDate) -> Option<(usize, Periode)> {
    let mot = |k: usize| s.get(i + k).map(String::as_str).unwrap_or("");
    let jour = |d: NaiveDate, n: u64, etiquette: &str| Periode {
        debut: Some(d),
        fin: d.checked_add_days(Days::new(n)),
        etiquette: etiquette.into(),
    };
    let lundi = aujourdhui - chrono::Duration::days(aujourdhui.weekday().num_days_from_monday() as i64);
    let ce_mois = premier_du_mois(aujourdhui.year(), aujourdhui.month());
    let cette_annee = premier_du_mois(aujourdhui.year(), 1);
    let mois_entier = |a: i32, m: u32| {
        let d = premier_du_mois(a, m);
        Periode { debut: Some(d), fin: d.checked_add_months(Months::new(1)), etiquette: format!("{} {a}", MOIS_LISIBLES[m as usize - 1]) }
    };
    // « la semaine dernière », « le mois dernier », « l'année dernière » : l'article est pris avec.
    let article = usize::from(matches!(mot(0), "la" | "le" | "l"));
    let passe = |k: usize| matches!(mot(k), "derniere" | "dernier" | "passee" | "passe");
    Some(match (mot(0), mot(1)) {
        ("aujourd", "hui") => (2, jour(aujourdhui, 1, "aujourd'hui")),
        ("aujourdhui", _) => (1, jour(aujourdhui, 1, "aujourd'hui")),
        ("hier", _) => (1, jour(aujourdhui.pred_opt()?, 1, "hier")),
        ("cette", "semaine") => (2, Periode { debut: Some(lundi), fin: None, etiquette: "cette semaine".into() }),
        ("ce", "mois" | "mois ci") => (2, Periode { debut: Some(ce_mois), fin: None, etiquette: "ce mois-ci".into() }),
        ("cette", "annee") => (2, Periode { debut: Some(cette_annee), fin: None, etiquette: "cette année".into() }),
        _ if mot(article) == "semaine" && passe(article + 1) => {
            (article + 2, jour(lundi - chrono::Duration::days(7), 7, "semaine dernière"))
        }
        _ if mot(article) == "mois" && passe(article + 1) => {
            let d = ce_mois.checked_sub_months(Months::new(1))?;
            (article + 2, Periode { debut: Some(d), fin: Some(ce_mois), etiquette: "mois dernier".into() })
        }
        _ if mot(article) == "annee" && passe(article + 1) => {
            let a = aujourdhui.year() - 1;
            (article + 2, Periode { debut: Some(premier_du_mois(a, 1)), fin: Some(cette_annee), etiquette: a.to_string() })
        }
        ("depuis" | "apres", _) => {
            let (n, p) = date(s, i + 1, aujourdhui)?;
            let etiquette = format!("depuis {}", p.etiquette);
            // « après 2020 » : à partir de 2021 ; « depuis 2020 » : à partir de 2020.
            let debut = if mot(0) == "apres" { p.fin } else { p.debut };
            (n + 1, Periode { debut, fin: None, etiquette: if mot(0) == "apres" { format!("après {}", p.etiquette) } else { etiquette } })
        }
        ("avant", _) => {
            let (n, p) = date(s, i + 1, aujourdhui)?;
            (n + 1, Periode { debut: None, fin: p.debut, etiquette: format!("avant {}", p.etiquette) })
        }
        (m, a) if mois(m).is_some() && annee(a, aujourdhui).is_some() => (2, mois_entier(annee(a, aujourdhui)?, mois(m)?)),
        // « en mars » : le dernier mois de mars passé (ou celui-ci).
        ("en", m) if mois(m).is_some() => {
            let m = mois(m)?;
            let a = if m <= aujourdhui.month() { aujourdhui.year() } else { aujourdhui.year() - 1 };
            (2, mois_entier(a, m))
        }
        (a, _) if annee(a, aujourdhui).is_some() => {
            let a = annee(a, aujourdhui)?;
            (1, Periode { debut: Some(premier_du_mois(a, 1)), fin: Some(premier_du_mois(a + 1, 1)), etiquette: a.to_string() })
        }
        _ => return None,
    })
}

/// (multiplicateur, étiquette)
fn unite(mot: &str) -> Option<(u64, &'static str)> {
    Some(match mot {
        "o" | "octet" | "octets" | "b" => (1, "o"),
        "k" | "ko" | "kb" => (1 << 10, "Ko"),
        "m" | "mo" | "mb" => (1 << 20, "Mo"),
        "g" | "go" | "gb" => (1 << 30, "Go"),
        "t" | "to" | "tb" => (1 << 40, "To"),
        _ => return None,
    })
}

/// Une taille à la position `i` (« > 1 Go », « plus de 500 Mo ») : (mots pris, plus grand ?, octets, étiquette).
fn taille(origine: &[String], s: &[String], i: usize) -> Option<(usize, bool, u64, String)> {
    let mot = |k: usize| s.get(i + k).map(String::as_str).unwrap_or("");
    let brut = |k: usize| origine.get(i + k).map(String::as_str).unwrap_or("");
    let (n, plus) = match (brut(0), mot(0), mot(1)) {
        (">" | "≥", _, _) => (1, true),
        ("<" | "≤", _, _) => (1, false),
        (_, "plus", "de") | (_, "superieur" | "superieure", "a") | (_, "au", "dessus") => (2, true),
        (_, "moins", "de") | (_, "inferieur" | "inferieure", "a") | (_, "en", "dessous") => (2, false),
        _ => return None,
    };
    // « au-dessus de », « en dessous de » : le « de » est pris avec.
    let n = if mot(n) == "de" { n + 1 } else { n };
    let nombre: f64 = brut(n).replace(',', ".").parse().ok().filter(|x: &f64| x.is_finite() && *x >= 0.0)?;
    let (facteur, nom) = unite(mot(n + 1))?;
    let etiquette = format!("{} {} {nom}", if plus { ">" } else { "<" }, brut(n));
    Some((n + 2, plus, (nombre * facteur as f64) as u64, etiquette))
}

/// Sépare les filtres du texte cherché, par rapport à aujourd'hui.
pub fn analyser(texte: &str) -> Analyse {
    analyser_au(texte, Local::now().date_naive())
}

pub fn analyser_au(texte: &str, aujourdhui: NaiveDate) -> Analyse {
    let texte = texte.trim();
    let origine = decouper(texte);
    let s: Vec<String> = origine.iter().map(|m| normaliser(m)).collect();
    let mut a = Analyse::default();
    let mut pris = vec![false; s.len()];
    let mut i = 0;
    let mut par_type = false;
    while i < s.len() {
        let mot = s[i].as_str();
        let n = if let Some((_, genres, etiquette)) = TYPES.iter().find(|(mots, _, _)| mots.contains(&mot)) {
            for g in genres.iter() {
                if !a.filtre.genres.contains(g) {
                    a.filtre.genres.push(g);
                }
            }
            if !a.etiquettes.iter().any(|e| e == etiquette) {
                a.etiquettes.push((*etiquette).into());
            }
            par_type = true;
            1
        } else if let Some((n, plus, octets, etiquette)) = taille(&origine, &s, i) {
            if plus {
                a.filtre.taille_min = Some(octets);
            } else {
                a.filtre.taille_max = Some(octets);
            }
            a.etiquettes.push(etiquette);
            n
        } else if let Some((n, p)) = date(&s, i, aujourdhui) {
            if let Some(d) = p.debut {
                a.filtre.depuis = Some(debut_du_jour(d));
            }
            if let Some(f) = p.fin {
                a.filtre.avant = Some(debut_du_jour(f));
            }
            a.etiquettes.push(p.etiquette);
            n
        } else if MODIFIE.contains(&mot) {
            1
        } else {
            0
        };
        for p in pris.iter_mut().skip(i).take(n) {
            *p = true;
        }
        i += n.max(1);
    }

    let mut reste: Vec<usize> = (0..s.len()).filter(|&k| !pris[k]).collect();
    // Seulement un type (« photos », « pdf ») : c'est sans doute un nom qu'on cherche (le dossier « Photos »).
    let seulement_un_type = par_type && a.filtre.depuis.is_none() && a.filtre.avant.is_none()
        && a.filtre.taille_min.is_none() && a.filtre.taille_max.is_none();
    while reste.first().is_some_and(|&k| LIAISONS.contains(&s[k].as_str())) {
        reste.remove(0);
    }
    while reste.last().is_some_and(|&k| LIAISONS.contains(&s[k].as_str())) {
        reste.pop();
    }
    if a.etiquettes.is_empty() || (seulement_un_type && reste.is_empty()) {
        return Analyse { reste: texte.to_string(), ..Default::default() };
    }
    a.reste = reste.iter().map(|&k| origine[k].as_str()).collect::<Vec<_>>().join(" ");
    a
}

/// « 3 mars 2024 » (heure locale).
pub fn date_lisible(secondes: i64) -> String {
    match Local.timestamp_opt(secondes, 0).earliest() {
        Some(t) => format!("{} {} {}", t.day(), MOIS_LISIBLES[t.month0() as usize], t.year()),
        None => String::new(),
    }
}

/// « 2,3 Mo », « 812 Ko », « 4 octets ».
pub fn taille_lisible(octets: u64) -> String {
    const UNITES: [&str; 4] = ["Ko", "Mo", "Go", "To"];
    if octets < 1024 {
        return format!("{octets} octet{}", if octets > 1 { "s" } else { "" });
    }
    let mut x = octets as f64 / 1024.0;
    let mut u = 0;
    while x >= 1024.0 && u < UNITES.len() - 1 {
        x /= 1024.0;
        u += 1;
    }
    let texte = if x < 10.0 { format!("{x:.1}") } else { format!("{x:.0}") };
    format!("{} {}", texte.replace('.', ","), UNITES[u])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn au(texte: &str) -> Analyse {
        // Un lundi : le 9 novembre 2026.
        analyser_au(texte, NaiveDate::from_ymd_opt(2026, 11, 9).unwrap())
    }

    fn jour(a: i32, m: u32, j: u32) -> i64 {
        debut_du_jour(NaiveDate::from_ymd_opt(a, m, j).unwrap())
    }

    #[test]
    fn types_et_texte() {
        let a = au("photos de vacances 2024");
        assert_eq!(a.reste, "vacances");
        assert_eq!(a.filtre.genres, vec!["photo"]);
        assert_eq!((a.filtre.depuis, a.filtre.avant), (Some(jour(2024, 1, 1)), Some(jour(2025, 1, 1))));
        assert_eq!(a.etiquettes, vec!["Photos", "2024"]);

        let a = au("pdf facture");
        assert_eq!(a.reste, "facture");
        assert_eq!(a.filtre.genres, vec!["pdf"]);

        // Un type seul : on cherche un nom, pas un filtre.
        let a = au("photos");
        assert_eq!(a.reste, "photos");
        assert!(a.filtre.est_vide());
        // Sans filtre, le texte reste tel quel.
        assert_eq!(au("rapport d'activité").reste, "rapport d'activité");
    }

    #[test]
    fn tailles() {
        let a = au("vidéos > 1 Go");
        assert_eq!(a.reste, "");
        assert_eq!(a.filtre.genres, vec!["video"]);
        assert_eq!(a.filtre.taille_min, Some(1 << 30));
        assert_eq!(a.etiquettes, vec!["Vidéos", "> 1 Go"]);
        let a = au("devis moins de 1,5Mo");
        assert_eq!(a.reste, "devis");
        assert_eq!(a.filtre.taille_max, Some(3 << 19));
        assert_eq!(au("films >700mo").filtre.taille_min, Some(700 << 20));
    }

    #[test]
    fn dates() {
        let a = au("modifié la semaine dernière");
        assert_eq!(a.reste, "");
        assert_eq!((a.filtre.depuis, a.filtre.avant), (Some(jour(2026, 11, 2)), Some(jour(2026, 11, 9))));
        let a = au("contrat du mois dernier");
        assert_eq!(a.reste, "contrat");
        assert_eq!((a.filtre.depuis, a.filtre.avant), (Some(jour(2026, 10, 1)), Some(jour(2026, 11, 1))));
        let a = au("photos mars 2023");
        assert_eq!((a.filtre.depuis, a.filtre.avant), (Some(jour(2023, 3, 1)), Some(jour(2023, 4, 1))));
        assert_eq!(a.etiquettes, vec!["Photos", "mars 2023"]);
        assert_eq!(au("notes d'hier").filtre.depuis, Some(jour(2026, 11, 8)));
        assert_eq!(au("cours aujourd'hui").reste, "cours");
        let a = au("factures avant 2020");
        assert_eq!((a.reste.as_str(), a.filtre.depuis, a.filtre.avant), ("factures", None, Some(jour(2020, 1, 1))));
        assert_eq!(au("photos en décembre").filtre.depuis, Some(jour(2025, 12, 1)));
        assert_eq!(au("l'année dernière").filtre.depuis, Some(jour(2025, 1, 1)));
        // Un nombre qui n'est pas une année reste cherché.
        assert_eq!(au("facture 48213").reste, "facture 48213");
    }

    #[test]
    fn lisible() {
        assert_eq!(taille_lisible(2_400_000), "2,3 Mo");
        assert_eq!(taille_lisible(812 * 1024), "812 Ko");
        assert_eq!(taille_lisible(4), "4 octets");
    }
}
