//! L'index : SQLite pour les fichiers et les morceaux, sqlite-vec pour les vecteurs, FTS5 pour les mots
//! exacts. Le texte des passages est gardé ici, sur le PC, pour les mots exacts et les extraits : jamais
//! dans les journaux.

use crate::pertinence::{self, Banque};
use crate::serveur::DIMENSION_INDEX;
use crate::vecteurs::IndexRapide;
use std::collections::HashMap;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::Once;

pub fn ouvrir(chemin: &Path) -> Result<Connection> {
    static EXTENSION: Once = Once::new();
    EXTENSION.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(sqlite_vec::sqlite3_vec_init as *const ())));
    });
    if let Some(dossier) = chemin.parent() {
        std::fs::create_dir_all(dossier)?;
    }
    let base = Connection::open(chemin)?;
    // L'appli et l'indexation écrivent parfois en même temps : on attend son tour plutôt que d'échouer.
    base.busy_timeout(std::time::Duration::from_secs(5))?;
    // Avec le journal WAL, « NORMAL » ne peut pas abîmer la base ; une coupure de courant peut seulement faire
    // perdre les tout derniers fichiers indexés, qui seront relus. Chaque fichier enregistré n'attend plus le disque.
    base.execute_batch("PRAGMA synchronous = NORMAL;")?;
    // Index d'une version précédente (vecteurs de 768 dimensions) : on repart de zéro, les noms restent.
    let ancien: Option<String> =
        base.query_row("SELECT sql FROM sqlite_master WHERE name = 'vecteurs'", [], |r| r.get(0)).optional()?;
    if ancien.is_some_and(|sql| !sql.contains(&format!("float[{DIMENSION_INDEX}]"))) {
        eprintln!("Index créé par une version précédente : il est refait (les vecteurs passent à {DIMENSION_INDEX} dimensions).");
        base.execute_batch("DROP TABLE vecteurs; DELETE FROM morceaux; DELETE FROM fichiers;")?;
    }
    base.execute_batch(&format!(
        "PRAGMA journal_mode = WAL;
         PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS fichiers (
             id INTEGER PRIMARY KEY,
             chemin TEXT UNIQUE NOT NULL,
             genre TEXT NOT NULL,
             taille INTEGER NOT NULL,
             modifie INTEGER NOT NULL,
             prep_ms REAL,
             modele_ms REAL,
             nb_morceaux INTEGER,
             pages_sans_texte INTEGER,
             appareil TEXT,
             erreur TEXT
         );
         CREATE TABLE IF NOT EXISTS morceaux (
             id INTEGER PRIMARY KEY,
             fichier_id INTEGER NOT NULL REFERENCES fichiers(id) ON DELETE CASCADE,
             debut REAL NOT NULL,
             fin REAL
         );
         CREATE INDEX IF NOT EXISTS morceaux_fichier ON morceaux(fichier_id);
         CREATE VIRTUAL TABLE IF NOT EXISTS vecteurs USING vec0(embedding float[{DIMENSION_INDEX}] distance_metric=cosine);
         CREATE TABLE IF NOT EXISTS sessions (
             id INTEGER PRIMARY KEY,
             date TEXT NOT NULL,
             appareil TEXT NOT NULL,
             nb_fichiers INTEGER NOT NULL,
             duree_s REAL NOT NULL,
             demarrage_s REAL NOT NULL,
             ram_programme INTEGER,
             ram_serveur INTEGER,
             vram INTEGER
         );"
    ))?;
    base.execute_batch(
        "CREATE TABLE IF NOT EXISTS noms (
             chemin TEXT PRIMARY KEY,
             cle TEXT NOT NULL
         );",
    )?;
    // Index créés par une version précédente : ajouter les colonnes manquantes.
    for (table, colonne, genre) in [
        ("morceaux", "nature", "TEXT"),
        ("fichiers", "ocr_ms", "REAL"),
        ("fichiers", "transcription_ms", "REAL"),
        ("fichiers", "transcrit", "INTEGER NOT NULL DEFAULT 0"),
        ("morceaux", "biais", "REAL"),
        ("fichiers", "visages_faits", "INTEGER NOT NULL DEFAULT 0"),
        ("morceaux", "partie", "TEXT"),
        ("fichiers", "exif_fait", "INTEGER NOT NULL DEFAULT 0"),
        ("fichiers", "lat", "REAL"),
        ("fichiers", "lon", "REAL"),
        ("fichiers", "prise", "INTEGER"),
        ("fichiers", "lieu_id", "INTEGER"),
        ("fichiers", "essai", "INTEGER"),
    ] {
        let existe: bool = base.query_row(
            &format!("SELECT COUNT(*) > 0 FROM pragma_table_info('{table}') WHERE name = '{colonne}'"),
            [],
            |r| r.get(0),
        )?;
        if !existe {
            base.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {colonne} {genre};"))?;
        }
    }
    // Les mots exacts (FTS5) et le journal que suit l'index rapide (vecteurs.rs). Les déclencheurs
    // tiennent le journal quel que soit le programme qui indexe, et vident les textes des morceaux retirés
    // (aussi quand ils partent avec leur fichier).
    let textes_existaient: bool =
        base.query_row("SELECT COUNT(*) > 0 FROM sqlite_master WHERE name = 'textes'", [], |r| r.get(0))?;
    base.execute_batch(
        "CREATE VIRTUAL TABLE IF NOT EXISTS textes USING fts5(texte, tokenize = 'unicode61 remove_diacritics 2');
         CREATE TABLE IF NOT EXISTS journal_vecteurs (
             seq INTEGER PRIMARY KEY AUTOINCREMENT,
             id INTEGER NOT NULL,
             op INTEGER NOT NULL
         );
         CREATE TRIGGER IF NOT EXISTS journal_ajout AFTER INSERT ON morceaux BEGIN
             INSERT INTO journal_vecteurs (id, op) VALUES (new.id, 1);
         END;
         CREATE TRIGGER IF NOT EXISTS journal_retrait AFTER DELETE ON morceaux BEGIN
             INSERT INTO journal_vecteurs (id, op) VALUES (old.id, 0);
             DELETE FROM textes WHERE rowid = old.id;
         END;",
    )?;
    // Les visages des photos (visages.rs) : leur place, leur empreinte, et le prénom tiré du nom du fichier.
    base.execute_batch(
        "CREATE TABLE IF NOT EXISTS visages (
             id INTEGER PRIMARY KEY,
             fichier_id INTEGER NOT NULL REFERENCES fichiers(id) ON DELETE CASCADE,
             x REAL, y REAL, l REAL, h REAL,
             empreinte BLOB NOT NULL,
             prenom TEXT
         );
         CREATE INDEX IF NOT EXISTS visages_fichier ON visages(fichier_id);",
    )?;
    // Les lieux des photos (lieux.rs) : ville, région, pays, et tous leurs noms (normalisés, séparés par |).
    base.execute_batch(
        "CREATE TABLE IF NOT EXISTS lieux (
             id INTEGER PRIMARY KEY,
             ville TEXT NOT NULL,
             region TEXT NOT NULL,
             pays TEXT NOT NULL,
             noms_ville TEXT NOT NULL,
             noms_region TEXT NOT NULL,
             noms_pays TEXT NOT NULL,
             UNIQUE (ville, region, pays)
         );
         CREATE INDEX IF NOT EXISTS fichiers_lieu ON fichiers(lieu_id);",
    )?;
    // Index d'avant les mots exacts : leur texte n'a pas été gardé. Les fichiers qui en ont sont relus une fois.
    if !textes_existaient {
        base.execute(
            "UPDATE fichiers SET modifie = -1 WHERE id IN (SELECT DISTINCT fichier_id FROM morceaux WHERE nature IN
             ('page', 'page_ocr', 'texte', 'diapo', 'feuille', 'ligne', 'texte_image', 'parole'))",
            [],
        )?;
    }
    Ok(base)
}

/// L'état d'un fichier de l'index.
pub struct Etat {
    pub id: i64,
    pub taille: i64,
    pub modifie: i64,
    pub sans_erreur: bool,
    /// Quand il a été lu pour la dernière fois (secondes depuis 1970 ; rien pour un index plus ancien).
    pub essai: Option<i64>,
}

/// L'état de tous les fichiers de l'index, en une seule lecture, par chemin.
pub fn etats(base: &Connection) -> Result<HashMap<String, Etat>> {
    let mut req = base.prepare("SELECT chemin, id, taille, modifie, erreur IS NULL, essai FROM fichiers")?;
    let etats = req
        .query_map([], |r| {
            Ok((r.get(0)?, Etat { id: r.get(1)?, taille: r.get(2)?, modifie: r.get(3)?, sans_erreur: r.get(4)?, essai: r.get(5)? }))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(etats)
}

/// Retire ces fichiers de l'index (supprimés ou déplacés sur le disque).
pub fn oublier(base: &mut Connection, ids: &[i64]) -> Result<()> {
    supprimer_fichiers(base, ids)
}

/// Vide la table des noms (l'appli cherche les noms dans la liste des disques, plus ici).
pub fn vider_noms(base: &Connection) -> Result<()> {
    if base.query_row("SELECT EXISTS(SELECT 1 FROM noms)", [], |r| r.get::<_, bool>(0))? {
        base.execute("DELETE FROM noms", [])?;
    }
    Ok(())
}

/// Vrai si le fichier est déjà dans l'index, sans erreur et inchangé.
pub fn a_jour(base: &Connection, chemin: &str, taille: u64, modifie: i64) -> Result<bool> {
    let ligne: Option<(i64, i64, Option<String>)> = base
        .query_row("SELECT taille, modifie, erreur FROM fichiers WHERE chemin = ?1", [chemin], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .optional()?;
    Ok(matches!(ligne, Some((t, m, None)) if t == taille as i64 && m == modifie))
}

pub struct Fiche<'a> {
    pub chemin: &'a str,
    pub genre: &'a str,
    pub taille: u64,
    pub modifie: i64,
    pub prep_ms: f64,
    pub ocr_ms: f64,
    pub transcription_ms: f64,
    pub modele_ms: f64,
    pub pages_sans_texte: usize,
    pub appareil: &'a str,
    pub erreur: Option<String>,
}

/// Un morceau indexé.
pub struct MorceauIndexe {
    pub nature: &'static str,
    pub debut: f64,
    pub fin: Option<f64>,
    /// Rien : morceau trouvable seulement par ses mots exacts (très gros fichiers de texte).
    pub vecteur: Option<Vec<f32>>,
    /// Le passage de texte (documents, OCR, transcriptions) : pour les mots exacts et les extraits.
    pub texte: Option<String>,
    /// Dans une archive : le fichier dont vient le morceau.
    pub partie: Option<String>,
}

/// Remplace le fichier et ses morceaux dans l'index.
pub fn enregistrer(base: &mut Connection, f: &Fiche, morceaux: &[MorceauIndexe], banque: &Banque) -> Result<()> {
    let t = base.transaction()?;
    let ancien: Option<i64> = t.query_row("SELECT id FROM fichiers WHERE chemin = ?1", [f.chemin], |r| r.get(0)).optional()?;
    if let Some(id) = ancien {
        t.execute("DELETE FROM vecteurs WHERE rowid IN (SELECT id FROM morceaux WHERE fichier_id = ?1)", [id])?;
        t.execute("DELETE FROM fichiers WHERE id = ?1", [id])?;
    }
    t.execute(
        "INSERT INTO fichiers (chemin, genre, taille, modifie, prep_ms, ocr_ms, transcription_ms, modele_ms, nb_morceaux,
                               pages_sans_texte, appareil, erreur, essai)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, CAST(strftime('%s', 'now') AS INTEGER))",
        params![f.chemin, f.genre, f.taille as i64, f.modifie, f.prep_ms, f.ocr_ms, f.transcription_ms, f.modele_ms,
                morceaux.len() as i64, f.pages_sans_texte as i64, f.appareil, f.erreur],
    )?;
    let fichier_id = t.last_insert_rowid();
    {
        let mut ajout_morceau =
            t.prepare("INSERT INTO morceaux (fichier_id, nature, debut, fin, biais, partie) VALUES (?1, ?2, ?3, ?4, ?5, ?6)")?;
        let mut ajout_vecteur = t.prepare("INSERT INTO vecteurs (rowid, embedding) VALUES (?1, ?2)")?;
        let mut ajout_texte = t.prepare("INSERT INTO textes (rowid, texte) VALUES (?1, ?2)")?;
        for m in morceaux {
            let biais = m.vecteur.as_ref().map(|v| banque.biais(v));
            ajout_morceau.execute(params![fichier_id, m.nature, m.debut, m.fin, biais, m.partie])?;
            let id = t.last_insert_rowid();
            if let Some(v) = &m.vecteur {
                ajout_vecteur.execute(params![id, octets(v)])?;
            }
            if let Some(texte) = &m.texte {
                ajout_texte.execute(params![id, texte])?;
            }
        }
    }
    t.commit()?;
    Ok(())
}

pub struct Resultat {
    /// Score par le sens, corrigé des « aimants » (0 pour un résultat trouvé seulement par ses mots).
    pub score: f32,
    /// Trouvé aussi par les mots exacts de la recherche.
    pub exact: bool,
    /// Le passage autour des mots trouvés ; ils sont entourés des caractères 0x01 et 0x02.
    pub extrait: Option<String>,
    pub chemin: String,
    pub genre: String,
    pub nature: Option<String>,
    pub debut: f64,
    pub fin: Option<f64>,
    /// Taille en octets et date (secondes depuis 1970) : de prise de vue pour une photo qui la connaît,
    /// de modification sinon.
    pub taille: u64,
    pub modifie: i64,
    /// Dans une archive : le fichier où c'est trouvé.
    pub partie: Option<String>,
}

impl Resultat {
    /// Où regarder dans le fichier : « page 4 », « dit 01:23–01:53 », « image 00:03–00:06 »…
    pub fn position(&self) -> String {
        let n = self.debut as u32;
        let moment = || self.fin.map(|f| format!("{}–{}", horodatage(self.debut), horodatage(f))).unwrap_or_default();
        // Index d'une très ancienne version : pas de nature, on la déduit du type.
        let dans = |p: String| match &self.partie {
            Some(partie) if p.is_empty() => format!("dans {partie}"),
            Some(partie) => format!("dans {partie}, {p}"),
            None => p,
        };
        let nature = self.nature.as_deref().unwrap_or(match self.genre.as_str() {
            "pdf" => "page",
            "audio" => "son",
            _ => "",
        });
        dans(match nature {
            "page" => format!("page {n}"),
            "page_ocr" => format!("page {n} (texte lu)"),
            "diapo" => format!("diapo {n}"),
            "feuille" => format!("feuille {n}"),
            "ligne" => format!("ligne {n}"),
            "texte_image" => "texte lu dans l'image".into(),
            "plan" => format!("image {}", moment()),
            "son" if self.genre == "video" => format!("son {}", moment()),
            "son" => moment(),
            "parole" => format!("dit {}", moment()),
            _ => String::new(),
        })
    }
}

/// « 01:23 », ou « 1:02:03 » au-delà d'une heure.
pub fn horodatage(s: f64) -> String {
    let s = s as u64;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{:02}:{:02}", s / 60, s % 60) }
}

/// Une recherche par le contenu.
pub struct Requete<'a> {
    /// Le vecteur de la phrase cherchée.
    pub vecteur: &'a [f32],
    /// La phrase elle-même, pour les mots exacts.
    pub texte: &'a str,
    pub n: usize,
    /// Type, date, taille (voir `filtres`).
    pub filtre: &'a crate::filtres::Filtre,
    /// Score par le sens minimal (les résultats trouvés par leurs mots exacts passent toujours).
    pub seuil: f32,
}

/// Mots trop courants pour compter dans une recherche par mots exacts.
const MOTS_VIDES: [&str; 46] = [
    "le", "la", "les", "un", "une", "des", "du", "de", "et", "ou", "en", "au", "aux", "sur", "sous", "pour", "par",
    "avec", "dans", "que", "qui", "quoi", "ce", "cet", "cette", "ces", "son", "sa", "ses", "mon", "ma", "mes", "est",
    "sont", "pas", "plus", "the", "of", "and", "to", "in", "for", "on", "with", "is", "it",
];
/// Au-delà de ce nombre de passages, des mots sont trop courants pour départager quoi que ce soit.
const MOTS_EXACTS_MAX: i64 = 15_000;
/// Fusion des classements (« reciprocal rank fusion ») : 60 est la valeur usuelle.
const FUSION_K: f32 = 60.0;

/// « Facture n° 2026-48213 » → `"facture"* "2026"* "48213"*` (tous les mots, chacun en début de mot).
fn requete_mots(texte: &str) -> Option<String> {
    let mots: Vec<String> = texte
        .split(|c: char| !c.is_alphanumeric())
        .map(|m| m.to_lowercase())
        .filter(|m| m.chars().count() >= 2 && !MOTS_VIDES.contains(&m.as_str()))
        .collect();
    (!mots.is_empty()).then(|| mots.iter().map(|m| format!("\"{m}\"*")).collect::<Vec<_>>().join(" "))
}

/// Les fichiers, natures et positions des morceaux `ids`.
fn infos(base: &Connection, ids: &[i64]) -> Result<HashMap<i64, (Resultat, Option<f32>)>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let liste = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
    let mut req = base.prepare(&format!(
        "SELECT m.id, f.chemin, f.genre, m.debut, m.fin, m.nature, m.biais, f.taille, COALESCE(f.prise, f.modifie), m.partie
         FROM morceaux m JOIN fichiers f ON f.id = m.fichier_id WHERE m.id IN ({liste})"
    ))?;
    let lignes = req.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            Resultat {
                score: 0.0,
                exact: false,
                extrait: None,
                chemin: r.get(1)?,
                genre: r.get(2)?,
                debut: r.get(3)?,
                fin: r.get(4)?,
                nature: r.get(5)?,
                taille: r.get::<_, i64>(7)? as u64,
                modifie: r.get(8)?,
                partie: r.get(9)?,
            },
            r.get::<_, Option<f64>>(6)?.map(|b| b as f32),
        ))
    })?;
    let mut sortie = HashMap::new();
    for l in lignes {
        let (id, res, biais) = l?;
        sortie.insert(id, (res, biais));
    }
    Ok(sortie)
}

/// Les `n` meilleurs fichiers, par le sens (au-dessus du seuil) et par les mots exacts, et le nombre de
/// fichiers proches mais sous le seuil. `index` : l'index rapide s'il est prêt (sinon, sqlite-vec).
pub fn chercher(base: &Connection, q: &Requete, index: Option<&IndexRapide>) -> Result<(Vec<Resultat>, usize)> {
    let garder = |r: &Resultat| {
        q.filtre.garde_genre(&r.genre) && q.filtre.garde_infos(r.taille, r.modifie) && q.filtre.garde_parmi(&r.chemin)
    };
    let filtre = !q.filtre.est_vide();

    // 1. Par le sens : large, puis reclassé avec le score corrigé des « aimants ». Avec un filtre, plus large
    // encore : beaucoup des plus proches seront écartés.
    let k = if filtre { 4096 } else { (q.n * 30).clamp(200, 4096) };
    let proches: Vec<(i64, f32)> = match index {
        Some(i) if !i.is_empty() => i.chercher(q.vecteur, k)?,
        _ => {
            let mut req = base.prepare("SELECT rowid, distance FROM vecteurs WHERE embedding MATCH ?1 AND k = ?2")?;
            req.query_map(params![octets(q.vecteur), k as i64], |r| Ok((r.get(0)?, r.get::<_, f64>(1)? as f32)))?
                .collect::<rusqlite::Result<_>>()?
        }
    };
    let mut details = infos(base, &proches.iter().map(|(id, _)| *id).collect::<Vec<_>>())?;
    let mut par_sens: Vec<Resultat> = proches
        .iter()
        .filter_map(|(id, distance)| {
            let (mut r, biais) = details.remove(id)?;
            r.score = pertinence::score(1.0 - distance, biais);
            garder(&r).then_some(r)
        })
        .collect();
    par_sens.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut vus = std::collections::HashSet::new();
    par_sens.retain(|r| vus.insert(r.chemin.clone()));
    let (par_sens, sous_le_seuil): (Vec<Resultat>, Vec<Resultat>) = par_sens.into_iter().partition(|r| r.score >= q.seuil);

    // 2. Par les mots exacts : chaque mot doit y être (au début d'un mot), avec l'extrait autour. Des mots
    // présents partout (« function return » : 99 000 passages) coûteraient 250 ms à classer pour rien : la
    // recherche par le sens s'en charge. Compter, lui, est rapide.
    let mut par_mots: Vec<Resultat> = Vec::new();
    let mots_utiles = requete_mots(q.texte).filter(|mots| {
        base.query_row(
            "SELECT COUNT(*) FROM (SELECT 1 FROM textes WHERE textes MATCH ?1 LIMIT ?2)",
            params![mots, MOTS_EXACTS_MAX + 1],
            |r| r.get::<_, i64>(0),
        )
        .is_ok_and(|n| n <= MOTS_EXACTS_MAX)
    });
    if let Some(mots) = mots_utiles {
        let trouves: Vec<(i64, String)> = {
            let mut req = base.prepare(
                "SELECT rowid, snippet(textes, 0, char(1), char(2), '…', 14) FROM textes
                 WHERE textes MATCH ?1 ORDER BY rank LIMIT ?2",
            )?;
            let limite = if filtre { 3000 } else { 300 };
            req.query_map(params![&mots, limite], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
        };
        let mut details = infos(base, &trouves.iter().map(|(id, _)| *id).collect::<Vec<_>>())?;
        let mut vus = std::collections::HashSet::new();
        for (id, extrait) in trouves {
            if let Some((mut r, _)) = details.remove(&id) {
                if garder(&r) && vus.insert(r.chemin.clone()) {
                    r.exact = true;
                    r.extrait = Some(extrait);
                    par_mots.push(r);
                }
            }
        }
    }

    // 3. Fusion : un fichier trouvé des deux façons passe devant. L'extrait vient des mots exacts.
    let mut fusion: HashMap<String, (f32, Resultat)> = HashMap::new();
    for (rang, r) in par_sens.into_iter().enumerate() {
        fusion.insert(r.chemin.clone(), (1.0 / (FUSION_K + rang as f32), r));
    }
    for (rang, r) in par_mots.into_iter().enumerate() {
        let points = 1.0 / (FUSION_K + rang as f32);
        match fusion.get_mut(&r.chemin) {
            Some((total, deja)) => {
                *total += points;
                let score = deja.score;
                *deja = Resultat { score, ..r };
            }
            None => {
                fusion.insert(r.chemin.clone(), (points, r));
            }
        }
    }
    let ecartes = sous_le_seuil.iter().filter(|r| !fusion.contains_key(&r.chemin)).count();
    let mut sortie: Vec<(f32, Resultat)> = fusion.into_values().collect();
    sortie.sort_by(|a, b| b.0.total_cmp(&a.0));
    Ok((sortie.into_iter().take(q.n).map(|(_, r)| r).collect(), ecartes))
}

/// Les `n` fichiers lus les plus récemment modifiés qui respectent `filtre` (une recherche faite seulement de
/// filtres : « photos 2024 », « vidéos > 1 Go »).
pub fn lister(base: &Connection, filtre: &crate::filtres::Filtre, n: usize) -> Result<Vec<Resultat>> {
    let mut req = base.prepare(
        "SELECT chemin, genre, taille, COALESCE(prise, modifie) AS date FROM fichiers WHERE erreur IS NULL
         AND date >= ?1 AND date < ?2 AND taille >= ?3 AND taille <= ?4 ORDER BY date DESC",
    )?;
    let bornes = params![
        filtre.depuis.unwrap_or(i64::MIN),
        filtre.avant.unwrap_or(i64::MAX),
        filtre.taille_min.unwrap_or(0) as i64,
        filtre.taille_max.map_or(i64::MAX, |t| t.min(i64::MAX as u64) as i64)
    ];
    let lignes = req.query_map(bornes, |r| {
        Ok(Resultat {
            score: 0.0,
            exact: false,
            extrait: None,
            chemin: r.get(0)?,
            genre: r.get(1)?,
            nature: None,
            debut: 0.0,
            fin: None,
            taille: r.get::<_, i64>(2)? as u64,
            modifie: r.get(3)?,
            partie: None,
        })
    })?;
    let mut sortie = Vec::new();
    for r in lignes {
        let r = r?;
        if filtre.garde_genre(&r.genre) && filtre.garde_parmi(&r.chemin) {
            sortie.push(r);
            if sortie.len() == n {
                break;
            }
        }
    }
    Ok(sortie)
}

#[cfg(test)]
mod tests {
    /// Vitesse de recherche quand l'index grossit (tout un PC) :
    /// cargo test --release vitesse_recherche -- --ignored --nocapture
    #[test]
    #[ignore]
    fn vitesse_recherche() {
        use std::time::Instant;
        let chemin = std::env::temp_dir().join("furet-vitesse.sqlite");
        let _ = std::fs::remove_file(&chemin);
        let mut base = super::ouvrir(&chemin).unwrap();
        let mut graine = 1u64;
        let mut hasard = move || {
            graine ^= graine << 13;
            graine ^= graine >> 7;
            graine ^= graine << 17;
            (graine >> 11) as f32 / (1u64 << 53) as f32 - 0.5
        };
        let mut vecteur = || {
            let v: Vec<f32> = (0..super::DIMENSION_INDEX).map(|_| hasard()).collect();
            let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            v.into_iter().map(|x| x / n).collect::<Vec<f32>>()
        };
        let mut total = 0;
        for palier in [50_000, 200_000, 500_000] {
            let t = Instant::now();
            let tr = base.transaction().unwrap();
            {
                let mut ajout = tr.prepare("INSERT INTO vecteurs (rowid, embedding) VALUES (?1, ?2)").unwrap();
                while total < palier {
                    total += 1;
                    ajout.execute(rusqlite::params![total, super::octets(&vecteur())]).unwrap();
                }
            }
            tr.commit().unwrap();
            let ajout_s = t.elapsed().as_secs_f64();
            let mut req = base.prepare("SELECT rowid, distance FROM vecteurs WHERE embedding MATCH ?1 AND k = 300").unwrap();
            let t = Instant::now();
            for _ in 0..10 {
                let n = req.query_map([super::octets(&vecteur())], |_| Ok(())).unwrap().count();
                assert_eq!(n, 300);
            }
            let taille = std::fs::metadata(&chemin).unwrap().len() >> 20;
            println!(
                "{palier:>7} vecteurs : recherche {:.0} ms, index {taille} Mo (ajout du palier {ajout_s:.1} s)",
                t.elapsed().as_secs_f64() * 100.0
            );
        }
        // Mesuré le 8 novembre 2026 (RTX 4070 Laptop, i7-12700H) : 66 ms à 50 000, 666 ms à 500 000.
        // En int8 : 440 ms ; en binaire : 140 ms mais moins précis ; binaire + reclassement : 945 ms.
        // Au-delà de ~100 000 morceaux, l'appli devra passer à un index HNSW (usearch).
        drop(base);
        let _ = std::fs::remove_file(&chemin);
    }
}

fn octets(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn depuis_octets(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

/// Remplace les noms enregistrés sous `dossier` par ceux trouvés maintenant (tous types de fichiers).
pub fn enregistrer_noms(base: &mut Connection, dossier: &str, chemins: &[String]) -> Result<()> {
    let t = base.transaction()?;
    t.execute("DELETE FROM noms WHERE substr(chemin, 1, ?1) = ?2", params![dossier.chars().count() as i64, dossier])?;
    {
        let mut ajout = t.prepare("INSERT OR REPLACE INTO noms (chemin, cle) VALUES (?1, ?2)")?;
        for c in chemins {
            let nom = std::path::Path::new(c).file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
            ajout.execute(params![c, crate::noms::normaliser(&nom)])?;
        }
    }
    t.commit()?;
    Ok(())
}

/// Les `n` fichiers dont le nom correspond le mieux, avec leur score.
pub fn chercher_noms(base: &Connection, recherche: &str, n: usize, genre: Option<&str>) -> Result<Vec<(f32, String)>> {
    let recherche = crate::noms::normaliser(recherche);
    let mut req = base.prepare("SELECT chemin, cle FROM noms")?;
    let mut trouves: Vec<(f32, String)> = req
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .filter_map(|r| r.ok())
        .filter(|(chemin, _)| {
            genre.is_none_or(|g| crate::fichiers::Genre::de(std::path::Path::new(chemin)).is_some_and(|x| x.nom() == g))
        })
        .filter_map(|(chemin, cle)| {
            let s = crate::noms::score(&recherche, &cle);
            (s > 0.0).then_some((s, chemin))
        })
        .collect();
    trouves.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.len().cmp(&b.1.len())));
    trouves.truncate(n);
    Ok(trouves)
}

/// Retire de l'index les fichiers de `dossier` qui ne sont plus sur le disque (supprimés, déplacés).
pub fn oublier_absents(base: &mut Connection, dossier: &str, presents: &std::collections::HashSet<String>) -> Result<usize> {
    let ids: Vec<(i64, String)> = {
        let mut req = base.prepare("SELECT id, chemin FROM fichiers WHERE substr(chemin, 1, ?1) = ?2")?;
        req.query_map(params![dossier.chars().count() as i64, dossier], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let absents: Vec<i64> = ids.into_iter().filter(|(_, c)| !presents.contains(c)).map(|(id, _)| id).collect();
    supprimer_fichiers(base, &absents)?;
    Ok(absents.len())
}

/// Retire de l'index tout ce qui est sous `dossier` (retiré des dossiers indexés).
pub fn oublier_dossier(base: &mut Connection, dossier: &str) -> Result<()> {
    let ids: Vec<i64> = {
        let mut req = base.prepare("SELECT id FROM fichiers WHERE substr(chemin, 1, ?1) = ?2")?;
        req.query_map(params![dossier.chars().count() as i64, dossier], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?
    };
    supprimer_fichiers(base, &ids)?;
    base.execute("DELETE FROM noms WHERE substr(chemin, 1, ?1) = ?2", params![dossier.chars().count() as i64, dossier])?;
    Ok(())
}

fn supprimer_fichiers(base: &mut Connection, ids: &[i64]) -> Result<()> {
    let t = base.transaction()?;
    for paquet in ids.chunks(500) {
        let liste = paquet.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
        t.execute(&format!("DELETE FROM vecteurs WHERE rowid IN (SELECT id FROM morceaux WHERE fichier_id IN ({liste}))"), [])?;
        t.execute(&format!("DELETE FROM fichiers WHERE id IN ({liste})"), [])?;
    }
    t.commit()?;
    Ok(())
}

/// Les noms (nom normalisé, chemin) des fichiers lus par leur contenu : pour la recherche par le nom avec
/// une faute de frappe tolérée (trop lente sur les millions de noms des disques entiers).
pub fn noms_des_fichiers_lus(base: &Connection) -> Result<Vec<(String, String)>> {
    let mut req = base.prepare("SELECT chemin FROM fichiers WHERE erreur IS NULL")?;
    let noms = req
        .query_map([], |r| {
            let chemin: String = r.get(0)?;
            let nom = Path::new(&chemin).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            Ok((crate::noms::normaliser(&nom), chemin))
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(noms)
}

pub fn nombre_noms(base: &Connection) -> Result<i64> {
    Ok(base.query_row("SELECT COUNT(*) FROM noms", [], |r| r.get(0))?)
}

/// Un audio ou une vidéo dont la parole n'est pas encore transcrite, avec ses tranches de son.
pub struct ATranscrire {
    pub fichier_id: i64,
    pub chemin: String,
    /// (début, fin, vecteur son)
    pub tranches: Vec<(f64, f64, Vec<f32>)>,
}

pub fn a_transcrire(base: &Connection) -> Result<Vec<ATranscrire>> {
    let mut req = base.prepare(
        "SELECT f.id, f.chemin, m.debut, m.fin, v.embedding
         FROM fichiers f
         JOIN morceaux m ON m.fichier_id = f.id AND m.nature = 'son'
         JOIN vecteurs v ON v.rowid = m.id
         WHERE f.genre IN ('audio', 'video') AND f.erreur IS NULL AND f.transcrit = 0
         ORDER BY f.id, m.debut",
    )?;
    let mut sortie: Vec<ATranscrire> = Vec::new();
    let lignes = req.query_map([], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, f64>(2)?, r.get::<_, Option<f64>>(3)?, r.get::<_, Vec<u8>>(4)?))
    })?;
    for l in lignes {
        let (id, chemin, debut, fin, v) = l?;
        if sortie.last().is_none_or(|d| d.fichier_id != id) {
            sortie.push(ATranscrire { fichier_id: id, chemin, tranches: Vec::new() });
        }
        sortie.last_mut().unwrap().tranches.push((debut, fin.unwrap_or(debut), depuis_octets(&v)));
    }
    Ok(sortie)
}

/// Ajoute les passages transcrits (début, fin, vecteur) et marque le fichier comme transcrit.
pub fn ajouter_paroles(
    base: &mut Connection,
    fichier_id: i64,
    paroles: &[(f64, f64, Vec<f32>, String)],
    ms: f64,
    banque: &Banque,
) -> Result<()> {
    let t = base.transaction()?;
    {
        let mut ajout_morceau =
            t.prepare("INSERT INTO morceaux (fichier_id, nature, debut, fin, biais) VALUES (?1, 'parole', ?2, ?3, ?4)")?;
        let mut ajout_vecteur = t.prepare("INSERT INTO vecteurs (rowid, embedding) VALUES (?1, ?2)")?;
        let mut ajout_texte = t.prepare("INSERT INTO textes (rowid, texte) VALUES (?1, ?2)")?;
        for (debut, fin, v, texte) in paroles {
            ajout_morceau.execute(params![fichier_id, debut, fin, banque.biais(v)])?;
            let id = t.last_insert_rowid();
            ajout_vecteur.execute(params![id, octets(v)])?;
            ajout_texte.execute(params![id, texte])?;
        }
    }
    t.execute(
        "UPDATE fichiers SET transcrit = 1, transcription_ms = ?2, nb_morceaux = nb_morceaux + ?3 WHERE id = ?1",
        params![fichier_id, ms, paroles.len() as i64],
    )?;
    t.commit()?;
    Ok(())
}
