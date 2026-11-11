//! La recherche : par le nom (tous les noms en mémoire, à chaque touche) et par le contenu (la phrase
//! devient un vecteur grâce au serveur léger, puis l'index). Rien de ce qu'on cherche ni des fichiers
//! trouvés n'est écrit dans les journaux.

use std::collections::{HashMap, VecDeque};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, bail};
use furet_moteur::disque::{self, ArbreNoms, Changement, IndexDisque};
use furet_moteur::fichiers::Genre;
use furet_moteur::noms::{IndexNoms, normaliser};
use furet_moteur::pertinence::Charabia;
use furet_moteur::serveur::{Client, Serveur};
use furet_moteur::vecteurs::{self, IndexRapide};
use furet_moteur::filtres::{self, date_lisible, taille_lisible};
use furet_moteur::{base, lieux, pertinence, visages};
use rusqlite::Connection;
use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use crate::{historique, raccourci, reglages};

/// Résultats affichés : par le nom au-dessus, par le contenu en dessous.
const NOMS_MAX: usize = 4;
const CONTENU_MAX: usize = 6;
/// Par le contenu, on regarde ce nombre de fichiers pour trouver les dossiers qui en rassemblent plusieurs…
const POUR_LES_DOSSIERS: usize = 40;
/// … au moins ce nombre ; et on en propose au plus deux.
const DOSSIER_MIN: usize = 3;
const DOSSIERS_MAX: usize = 2;
/// Avec un filtre (« photos 2024 vacances »), on regarde ce nombre de noms pour en garder quelques-uns.
const NOMS_A_FILTRER: usize = 400;
/// Sans filtre, ce nombre de noms est reclassé (fichiers ouverts, dossiers de l'utilisateur, nom exact).
const NOMS_A_RECLASSER: usize = 40;
/// Score minimal d'un nom (1 : le nom commence par la recherche ; 0,7 : elle est au milieu d'un mot ;
/// -0,2 dans les dossiers des programmes et du système).
const NOM_SCORE_MIN: f32 = 0.6;
/// Fichiers récents proposés quand la barre est vide.
const RECENTS_MAX: usize = 6;
/// Vecteurs des dernières recherches gardés : effacer une lettre et la retaper ne refait pas le calcul.
const VECTEURS_GARDES: usize = 128;
/// Le serveur démarre avec l'appli (~1,5 s) : une recherche lancée avant l'attend jusque-là.
const ATTENTE_SERVEUR_MAX: Duration = Duration::from_secs(8);
/// Au-delà, on oublie les fichiers proposés (l'interface ne garde que les derniers résultats).
const OFFERTS_MAX: usize = 5_000;
/// L'index rapide rattrape le journal à ce rythme (pendant une indexation, ~40 morceaux par seconde)…
const RATTRAPAGE: Duration = Duration::from_secs(2);
/// … par lots de cette taille (lus sans bloquer les recherches)…
const RATTRAPAGE_LOT: usize = 5_000;
/// … et se sauvegarde au plus à ce rythme s'il a changé (et à la fermeture).
const SAUVEGARDE: Duration = Duration::from_secs(120);
/// La liste des noms des disques est refaite à ce rythme (le suivi en direct comble entre-temps).
const NOUVELLE_LISTE: Duration = Duration::from_secs(6 * 3600);

struct Etat {
    racine: PathBuf,
    chemin_base: PathBuf,
    noms: RwLock<Arc<IndexNoms>>,
    /// Date de l'index quand les noms ont été chargés : on les recharge s'il a changé.
    date_noms: Mutex<Option<SystemTime>>,
    /// Fichiers lus par leur contenu, compté avec les noms.
    fichiers: Mutex<i64>,
    lecture: Mutex<Option<Connection>>,
    client: OnceLock<Client>,
    serveur: Mutex<Option<Serveur>>,
    /// L'index rapide (HNSW) : None tant qu'il se charge (la recherche passe alors par sqlite-vec).
    rapide: RwLock<Option<IndexRapide>>,
    /// Il a changé depuis sa dernière sauvegarde.
    rapide_modifie: AtomicBool,
    /// Les noms de tous les disques, suivis en direct : None tant qu'aucune liste n'est prête
    /// (la recherche par le nom se contente alors des dossiers indexés).
    disque: RwLock<Option<Arc<IndexDisque>>>,
    /// Windows n'a pas pu tout signaler (trop de changements d'un coup) : refaire la liste.
    disque_a_refaire: AtomicBool,
    /// Les fichiers proposés à l'interface, par clé : elle ne peut ouvrir que ceux-là.
    offerts: Mutex<HashMap<String, PathBuf>>,
    /// Pour relever le seuil des recherches qui ressemblent à des mots inventés.
    charabia: OnceLock<Charabia>,
    /// Les vecteurs des dernières recherches.
    vecteurs: Mutex<(HashMap<String, Vec<f32>>, VecDeque<String>)>,
}

/// Le numéro de la dernière recherche par le contenu : une plus ancienne, dépassée, s'arrête au plus tôt.
static DERNIERE: AtomicU64 = AtomicU64::new(0);

static ETAT: OnceLock<Etat> = OnceLock::new();

fn etat() -> &'static Etat {
    ETAT.get().expect("recherche::demarrer est appelé au lancement")
}

/// Charge les noms et démarre le serveur de recherche, chacun sur son fil : l'appli est prête tout de suite.
/// Au premier lancement, le serveur attend que ses outils et son modèle soient téléchargés.
pub fn demarrer(racine: PathBuf) {
    furet_moteur::processus::definir_racine(&racine);
    let chemin_base = racine.join("donnees").join("index.sqlite");
    let _ = ETAT.set(Etat {
        racine,
        chemin_base,
        noms: RwLock::new(Arc::new(IndexNoms::nouveau(Vec::new()))),
        date_noms: Mutex::new(None),
        fichiers: Mutex::new(0),
        lecture: Mutex::new(None),
        client: OnceLock::new(),
        serveur: Mutex::new(None),
        rapide: RwLock::new(None),
        rapide_modifie: AtomicBool::new(false),
        disque: RwLock::new(None),
        disque_a_refaire: AtomicBool::new(false),
        offerts: Mutex::new(HashMap::new()),
        charabia: OnceLock::new(),
        vecteurs: Mutex::new(Default::default()),
    });
    std::thread::spawn(charger_noms);
    std::thread::spawn(entretenir_index_rapide);
    std::thread::spawn(entretenir_noms_des_disques);
    std::thread::spawn(|| {
        let e = etat();
        crate::installation::attendre_essentiel();
        match Serveur::demarrer_requetes(&e.racine, &e.chemin_base.with_file_name("requetes.log")) {
            Ok(s) => {
                log::info!("recherche par le contenu prête en {} ms", s.demarrage.as_millis());
                let client = s.client();
                let _ = e.client.set(client.clone());
                *verrou(&e.serveur) = Some(s);
                match Charabia::charger(&client) {
                    Ok(c) => {
                        let _ = e.charabia.set(c);
                    }
                    Err(err) => log::warn!("seuil adapté indisponible : {err:#}"),
                }
            }
            Err(err) => log::error!("serveur de recherche indisponible : {err:#}"),
        }
    });
}

/// Le dossier de Furet (outils, modèles) et le fichier de l'index.
pub fn racine_et_base() -> (PathBuf, PathBuf) {
    let e = etat();
    (e.racine.clone(), e.chemin_base.clone())
}

/// Arrête le serveur de recherche et sauvegarde l'index rapide (à la fermeture de l'appli).
pub fn arreter() {
    if let Some(e) = ETAT.get() {
        verrou(&e.serveur).take();
        if e.rapide_modifie.swap(false, Ordering::Relaxed) {
            sauver_index_rapide(e);
        }
    }
}

/// Les noms de tous les disques fixes : la liste sauvegardée tout de suite, le suivi en direct, puis une
/// liste neuve en arrière-plan (et de nouveau toutes les 6 heures, ou si Windows n'a pas tout pu dire).
fn entretenir_noms_des_disques() {
    let e = etat();
    let fichier = e.chemin_base.with_file_name("noms-disques.bin");
    let racines = disque::disques_fixes();
    let debut = Instant::now();
    let sauvegardee = ArbreNoms::charger(&fichier).ok();
    let deja_prete = sauvegardee.is_some();
    let index = Arc::new(IndexDisque::nouveau(sauvegardee.unwrap_or_else(ArbreNoms::vide)));
    if deja_prete {
        log::info!("noms des disques : {} rechargés en {} ms", index.len(), debut.elapsed().as_millis());
        *e.disque.write().unwrap_or_else(|e| e.into_inner()) = Some(index.clone());
    }
    // Le suivi démarre avant la nouvelle liste : rien de ce qui change pendant qu'elle se fait n'est perdu.
    for racine in racines.clone() {
        let index = index.clone();
        std::thread::Builder::new()
            .name("furet-suivi-disque".into())
            .spawn(move || {
                loop {
                    let fin = disque::surveiller(&racine, |c| {
                        if matches!(c, Changement::Debordement) {
                            etat().disque_a_refaire.store(true, Ordering::Relaxed);
                        }
                        index.noter(c);
                    });
                    if let Err(err) = fin {
                        log::warn!("suivi d'un disque interrompu : {err:#}");
                    }
                    // Disque débranché, erreur passagère : on réessaie plus tard.
                    std::thread::sleep(Duration::from_secs(30));
                }
            })
            .ok();
    }
    loop {
        let depuis = Instant::now();
        let arbre = disque::parcourir(&racines);
        log::info!("noms des disques : {} en {} s", arbre.len(), depuis.elapsed().as_secs());
        index.remplacer(arbre, depuis);
        *e.disque.write().unwrap_or_else(|e| e.into_inner()) = Some(index.clone());
        if let Err(err) = index.sauver(&fichier) {
            log::warn!("noms des disques non sauvegardés : {err:#}");
        }
        e.disque_a_refaire.store(false, Ordering::Relaxed);
        // Jusqu'à la prochaine liste : 6 heures, un débordement, ou trop de changements notés à part.
        while depuis.elapsed() < NOUVELLE_LISTE
            && !e.disque_a_refaire.load(Ordering::Relaxed)
            && index.changements() < disque::CHANGEMENTS_MAX
        {
            std::thread::sleep(Duration::from_secs(30));
        }
    }
}

fn fichier_index_rapide(e: &Etat) -> PathBuf {
    e.chemin_base.with_file_name("vecteurs.usearch")
}

fn sauver_index_rapide(e: &Etat) {
    let Ok(base) = base::ouvrir(&e.chemin_base) else { return };
    let garde = e.rapide.read().unwrap_or_else(|e| e.into_inner());
    if let Some(index) = garde.as_ref()
        && let Err(err) = index.sauver(&base, &fichier_index_rapide(e))
    {
        log::warn!("index rapide non sauvegardé : {err:#}");
    }
}

/// Charge l'index rapide (ou le construit), puis le tient à jour d'après le journal de SQLite.
fn entretenir_index_rapide() {
    let e = etat();
    let base = match base::ouvrir(&e.chemin_base) {
        Ok(b) => b,
        Err(err) => return log::error!("index rapide : base illisible : {err:#}"),
    };
    let fichier = fichier_index_rapide(e);
    let debut = Instant::now();
    let index = match IndexRapide::charger(&fichier) {
        Ok(i) => i,
        Err(_) => match IndexRapide::construire(&base) {
            Ok(i) => {
                e.rapide_modifie.store(true, Ordering::Relaxed);
                i
            }
            Err(err) => return log::error!("index rapide impossible : {err:#}"),
        },
    };
    log::info!(
        "index rapide prêt : {} morceaux, {} Mo, en {} ms",
        index.len(),
        index.memoire() >> 20,
        debut.elapsed().as_millis()
    );
    *e.rapide.write().unwrap_or_else(|e| e.into_inner()) = Some(index);
    let mut sauvegarde = Instant::now();
    loop {
        std::thread::sleep(RATTRAPAGE);
        let seq = e.rapide.read().unwrap_or_else(|e| e.into_inner()).as_ref().map_or(0, |i| i.seq());
        let rattrape = (|| -> anyhow::Result<bool> {
            let retard = vecteurs::en_attente(&base, seq)?;
            if retard > vecteurs::RATTRAPAGE_MAX {
                // Beaucoup de retard (une grosse indexation pendant que l'appli était fermée) : on reconstruit à côté.
                let neuf = IndexRapide::construire(&base)?;
                *e.rapide.write().unwrap_or_else(|e| e.into_inner()) = Some(neuf);
                return Ok(true);
            }
            if retard == 0 {
                return Ok(false);
            }
            // La lecture (lente) se fait sans verrou ; seule l'application, rapide, bloque les recherches.
            let lot = vecteurs::changements(&base, seq, RATTRAPAGE_LOT)?;
            if let Some(index) = e.rapide.write().unwrap_or_else(|e| e.into_inner()).as_mut() {
                index.appliquer(lot)?;
            }
            Ok(true)
        })();
        match rattrape {
            Ok(true) => e.rapide_modifie.store(true, Ordering::Relaxed),
            Ok(false) => {}
            Err(err) => log::warn!("index rapide : {err:#}"),
        }
        if sauvegarde.elapsed() > SAUVEGARDE && e.rapide_modifie.swap(false, Ordering::Relaxed) {
            sauver_index_rapide(e);
            sauvegarde = Instant::now();
        }
    }
}

fn verrou<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// La date de l'index : la base ou son journal SQLite, le plus récent.
fn date_index(e: &Etat) -> Option<SystemTime> {
    ["", "-wal"]
        .iter()
        .filter_map(|s| std::fs::metadata(format!("{}{s}", e.chemin_base.display())).ok()?.modified().ok())
        .max()
}

fn avec_lecture<R>(f: impl FnOnce(&Connection) -> anyhow::Result<R>) -> anyhow::Result<R> {
    let e = etat();
    let mut garde = verrou(&e.lecture);
    if garde.is_none() {
        *garde = Some(base::ouvrir(&e.chemin_base).context("ouverture de l'index")?);
    }
    f(garde.as_ref().expect("ouverte juste au-dessus"))
}

fn charger_noms() {
    let e = etat();
    let date = date_index(e);
    let debut = Instant::now();
    let lu = avec_lecture(|b| {
        let fichiers: i64 = b.query_row("SELECT COUNT(*) FROM fichiers WHERE erreur IS NULL", [], |r| r.get(0))?;
        Ok((base::noms_des_fichiers_lus(b)?, fichiers))
    });
    match lu {
        Ok((noms, fichiers)) => {
            let index = IndexNoms::nouveau(noms);
            log::info!("{} noms en mémoire, {fichiers} fichiers lus par leur contenu ({} ms)", index.len(), debut.elapsed().as_millis());
            *e.noms.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(index);
            *verrou(&e.fichiers) = fichiers;
            *verrou(&e.date_noms) = date;
        }
        Err(err) => log::error!("noms illisibles : {err:#}"),
    }
}

/// Recharge les noms si l'index a changé depuis (une indexation a eu lieu) : sur un fil à part.
pub fn rafraichir_si_change() {
    std::thread::spawn(|| {
        let e = etat();
        if date_index(e) != *verrou(&e.date_noms) {
            charger_noms();
        }
    });
}

/// Mirroir de `Ligne` dans `src/ipc/recherche.ts`.
#[derive(Debug, Clone, Serialize)]
pub struct Ligne {
    /// Ce qui désigne le fichier pour `ouvrir_fichier`.
    cle: String,
    nom: String,
    dossier: String,
    /// photo, pdf, audio, video, document, tableur, texte, code, dossier, ou fichier.
    genre: &'static str,
    /// Où regarder : « page 4 », « dit 01:23–01:53 »… (vide pour les résultats par le nom).
    position: String,
    /// L'adresse de sa miniature Windows.
    miniature: String,
    /// Le passage où ses mots ont été trouvés (mots entourés de 0x01 et 0x02), s'il y en a un.
    extrait: Option<String>,
    /// La page, la ligne ou le moment (secondes) trouvé, pour ouvrir l'aperçu au bon endroit.
    repere: Option<f64>,
}

/// Le chemin d'un fichier proposé à l'interface (pour sa miniature).
pub fn chemin_offert(cle: &str) -> Option<PathBuf> {
    verrou(&etat().offerts).get(cle).cloned()
}

/// Miroir de `EtatIndex` dans `src/ipc/recherche.ts`.
#[derive(Debug, Clone, Serialize)]
pub struct EtatIndex {
    /// Fichiers lus par leur contenu.
    fichiers: i64,
    noms: usize,
    /// La recherche par le contenu est prête.
    pret: bool,
}

#[tauri::command]
pub fn etat_index() -> EtatIndex {
    let e = etat();
    EtatIndex {
        fichiers: *verrou(&e.fichiers),
        noms: e.noms.read().unwrap_or_else(|e| e.into_inner()).len(),
        pret: e.client.get().is_some(),
    }
}

fn ligne(chemin: &str, position: String, extrait: Option<String>, dossier: bool, repere: Option<f64>) -> Ligne {
    let p = Path::new(chemin);
    let mut h = DefaultHasher::new();
    chemin.hash(&mut h);
    let cle = format!("{:016x}", h.finish());
    let mut offerts = verrou(&etat().offerts);
    if offerts.len() > OFFERTS_MAX {
        offerts.clear();
    }
    offerts.insert(cle.clone(), p.to_path_buf());
    Ligne {
        nom: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        dossier: p.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default(),
        genre: if dossier { "dossier" } else { Genre::de(p).map(|g| g.nom()).unwrap_or("fichier") },
        position,
        extrait,
        repere,
        miniature: crate::miniatures::adresse(&cle),
        cle,
    }
}

/// Par le nom : tous les disques (quelques millisecondes pour près de 2 millions de noms) ; s'il manque des
/// résultats, les dossiers indexés avec une faute de frappe tolérée (trop lent sur les disques entiers).
/// Sur un fil à part : l'interface ne se fige jamais, même quand un filtre fait lire le disque.
#[tauri::command]
pub async fn chercher_noms(texte: String) -> Vec<Ligne> {
    tauri::async_runtime::spawn_blocking(move || noms(&texte)).await.unwrap_or_default()
}

/// Tous les disques, puis s'il en manque, les dossiers indexés avec une faute de frappe tolérée ; le tout
/// reclassé. Avec un filtre, les noms sont cherchés plus largement puis triés (type, et date ou taille lues
/// sur le disque).
fn noms(texte: &str) -> Vec<Ligne> {
    let e = etat();
    let analyse = filtres::analyser(texte);
    let (texte, filtre) = (&analyse.reste, &analyse.filtre);
    if texte.is_empty() {
        return Vec::new();
    }
    let large = if filtre.est_vide() { NOMS_A_RECLASSER } else { NOMS_A_FILTRER };
    let disque = e.disque.read().unwrap_or_else(|e| e.into_inner()).clone();
    // (score, chemin, dossier)
    let mut candidats: Vec<(f32, PathBuf, bool)> = match &disque {
        Some(d) if !d.is_empty() => d
            .chercher(texte, large)
            .into_iter()
            .filter(|t| filtre.garde_chemin(&t.chemin, t.dossier))
            .take(NOMS_A_RECLASSER)
            .map(|t| (t.score, t.chemin, t.dossier))
            .collect(),
        _ => Vec::new(),
    };
    if candidats.len() < NOMS_MAX {
        let index = e.noms.read().unwrap_or_else(|e| e.into_inner()).clone();
        for (score, chemin) in index.chercher(texte, large, |_| true) {
            let chemin = PathBuf::from(chemin);
            if filtre.garde_chemin(&chemin, false) && !candidats.iter().any(|(_, c, _)| *c == chemin) {
                candidats.push((score, chemin, false));
            }
        }
    }
    reclasser(texte, &mut candidats);
    candidats
        .into_iter()
        // Le mot cherché au milieu d'un autre, dans les dossiers des programmes (« systemmanufacturers.h » pour
        // « facture ») : jamais ce qu'on cherche. Mieux vaut rien.
        .filter(|(score, _, _)| *score >= NOM_SCORE_MIN)
        .take(NOMS_MAX)
        .map(|(_, c, dossier)| ligne(&c.to_string_lossy(), String::new(), None, dossier, None))
        .collect()
}

/// Affine le classement par le nom : les fichiers souvent ouverts depuis Furet d'abord, puis ceux des
/// dossiers indexés et du dossier de l'utilisateur, et le nom exact (« budget » → « budget.xlsx » avant
/// « budget prévisionnel 2019.xlsx »). Rien n'est lu sur le disque.
fn reclasser(texte: &str, candidats: &mut [(f32, PathBuf, bool)]) {
    let cle = normaliser(texte);
    let dossiers = reglages::lire().dossiers;
    let maison = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    for (score, chemin, _) in candidats.iter_mut() {
        *score += historique::bonus(chemin);
        if dossiers.iter().any(|d| chemin.starts_with(d)) {
            *score += 0.04;
        } else if maison.as_ref().is_some_and(|m| chemin.starts_with(m)) {
            *score += 0.02;
        }
        if chemin.file_stem().is_some_and(|s| normaliser(&s.to_string_lossy()) == cle) {
            *score += 0.05;
        }
    }
    candidats.sort_by(|a, b| b.0.total_cmp(&a.0));
}

/// « à l'instant », « il y a 5 min », « il y a 2 h », « hier », « le 3 mars 2024 ».
fn quand(secondes: u64) -> String {
    let maintenant = SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    match maintenant.saturating_sub(secondes) {
        0..60 => "à l'instant".into(),
        s @ 60..3600 => format!("il y a {} min", s / 60),
        s @ 3600..86_400 => format!("il y a {} h", s / 3600),
        86_400..172_800 => "hier".into(),
        _ => format!("le {}", date_lisible(secondes as i64)),
    }
}

/// Les fichiers ouverts récemment depuis Furet, pour la barre vide.
#[tauri::command]
pub async fn recents() -> Vec<Ligne> {
    tauri::async_runtime::spawn_blocking(|| {
        historique::recents(RECENTS_MAX)
            .into_iter()
            .map(|(c, moment)| ligne(&c.to_string_lossy(), format!("ouvert {}", quand(moment)), None, c.is_dir(), None))
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Miroir de `Filtres` dans `src/ipc/recherche.ts`.
#[derive(Debug, Clone, Serialize)]
pub struct Filtres {
    /// Les filtres compris : « Photos », « 2024 », « > 1 Go ».
    etiquettes: Vec<String>,
    /// Ce qui reste à chercher (vide : seulement des filtres).
    reste: String,
}

/// Les filtres tapés dans la barre, pour les montrer ; avec le lieu nommé s'il y a des photos prises là.
#[tauri::command]
pub async fn filtres(texte: String) -> Filtres {
    let a = filtres::analyser(&texte);
    let mut etiquettes = a.etiquettes;
    if !a.reste.is_empty()
        && let Ok(Some(lieu)) = avec_lecture(|b| lieux::reperer(b, &a.reste))
    {
        etiquettes.push(lieu.etiquette);
    }
    Filtres { etiquettes, reste: a.reste }
}

/// « pris le 3 mars 2024 à Barcelona, Espagne » pour une photo de l'index qui le sait.
pub fn prise_et_lieu(chemin: &Path) -> Option<String> {
    let c = chemin.to_string_lossy();
    let (prise, lieu) = avec_lecture(|b| {
        let prise: Option<i64> = b.query_row("SELECT prise FROM fichiers WHERE chemin = ?1", [&c], |r| r.get(0)).unwrap_or(None);
        Ok((prise, lieux::lieu_de(b, &c)?))
    })
    .ok()?;
    match (prise, lieu) {
        (Some(p), Some(l)) => Some(format!("pris le {} à {l}", date_lisible(p))),
        (Some(p), None) => Some(format!("pris le {}", date_lisible(p))),
        (None, Some(l)) => Some(format!("pris à {l}")),
        (None, None) => None,
    }
}

/// Par le contenu : sur un fil à part, pour ne jamais bloquer l'interface.
#[tauri::command]
pub async fn chercher_contenu(texte: String) -> Result<Vec<Ligne>, String> {
    tauri::async_runtime::spawn_blocking(move || contenu(&texte))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

/// Le vecteur d'une recherche, gardé pour les suivantes.
fn vecteur(client: &Client, texte: &str) -> anyhow::Result<Vec<f32>> {
    let e = etat();
    if let Some(v) = verrou(&e.vecteurs).0.get(texte) {
        return Ok(v.clone());
    }
    let v = client.vecteur_requete(texte)?;
    let mut garde = verrou(&e.vecteurs);
    let (table, ordre) = &mut *garde;
    if table.insert(texte.to_owned(), v.clone()).is_none() {
        ordre.push_back(texte.to_owned());
        while ordre.len() > VECTEURS_GARDES {
            if let Some(vieux) = ordre.pop_front() {
                table.remove(&vieux);
            }
        }
    }
    Ok(v)
}

fn contenu(texte: &str) -> anyhow::Result<Vec<Ligne>> {
    let e = etat();
    let numero = DERNIERE.fetch_add(1, Ordering::Relaxed) + 1;
    let depassee = || DERNIERE.load(Ordering::Relaxed) != numero;
    let debut = Instant::now();
    let client = loop {
        if let Some(c) = e.client.get() {
            break c.clone();
        }
        if debut.elapsed() > ATTENTE_SERVEUR_MAX {
            bail!("la recherche par le contenu n'est pas prête");
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    let analyse = filtres::analyser(texte);
    let (texte, filtre) = (analyse.reste.as_str(), &analyse.filtre);
    if texte.is_empty() {
        // Seulement des filtres (« photos 2024 ») : les fichiers lus les plus récents qui les respectent.
        let trouves = avec_lecture(|b| base::lister(b, filtre, CONTENU_MAX + NOMS_MAX))?;
        return Ok(trouves
            .into_iter()
            .map(|r| {
                let position = format!("modifié le {} · {}", date_lisible(r.modifie), taille_lisible(r.taille));
                ligne(&r.chemin, position, None, false, None)
            })
            .collect());
    }
    let vecteur = vecteur(&client, texte)?;
    // Une autre recherche est partie entre-temps (on tape vite) : celle-ci ne sera pas affichée.
    if depassee() {
        bail!("recherche dépassée");
    }
    let seuil = e.charabia.get().map_or(pertinence::SEUIL, |c| c.seuil(&vecteur));
    // Plus qu'affiché : pour repérer les dossiers, et l'interface retire ceux déjà trouvés par leur nom.
    let requete = base::Requete { vecteur: &vecteur, texte, n: POUR_LES_DOSSIERS, filtre, seuil };
    let rapide = e.rapide.read().unwrap_or_else(|e| e.into_inner());
    let (trouves, personnes, au_lieu) = avec_lecture(|b| {
        let trouves = base::chercher(b, &requete, rapide.as_ref())?.0;
        // Une personne nommée : les photos où l'on reconnaît son visage, même sans son nom.
        let personnes = visages::chercher_personne(b, texte, CONTENU_MAX)?;
        // Un lieu où des photos ont été prises (« vacances Espagne ») : ces photos, les plus proches du
        // reste de la recherche d'abord (ou les plus récentes s'il n'y a que le lieu).
        let au_lieu = match lieux::reperer(b, texte)? {
            Some(lieu) => {
                let ici = filtres::Filtre { parmi: Some(lieux::photos(b, &lieu)?), ..filtre.clone() };
                let photos = if lieu.reste.is_empty() {
                    base::lister(b, &ici, CONTENU_MAX)?
                } else {
                    let r = base::Requete { vecteur: &vecteur, texte: &lieu.reste, n: CONTENU_MAX, filtre: &ici, seuil: 0.0 };
                    base::chercher(b, &r, rapide.as_ref())?.0
                };
                photos.into_iter().map(|p| (p.chemin, lieu.etiquette.clone())).collect()
            }
            None => Vec::new(),
        };
        Ok((trouves, personnes, au_lieu))
    })?;
    drop(rapide);

    let mut lignes: Vec<Ligne> = personnes
        .into_iter()
        .filter(|p| filtre.garde_chemin(Path::new(&p.chemin), false))
        .map(|p| ligne(&p.chemin, format!("{} · visage reconnu", majuscule(&p.prenom)), None, false, None))
        .collect();
    for (chemin, lieu) in au_lieu {
        let l = ligne(&chemin, format!("pris à {lieu}"), None, false, None);
        if !lignes.iter().any(|x| x.cle == l.cle) {
            lignes.push(l);
        }
    }
    let dossiers = dossiers_communs(trouves.iter().map(|r| r.chemin.as_str()));
    let mut deja: std::collections::HashSet<String> = lignes.iter().map(|l| l.cle.clone()).collect();
    let mut copies = std::collections::HashSet::new();
    for r in trouves {
        // Le dossier passe juste avant le premier de ses fichiers.
        if let Some(parent) = Path::new(&r.chemin).parent()
            && let Some((dossier, n)) = dossiers.iter().find(|(d, _)| d == parent)
        {
            let l = ligne(&dossier.to_string_lossy(), format!("{n} fichiers correspondants"), None, true, None);
            if deja.insert(l.cle.clone()) {
                lignes.push(l);
            }
        }
        let position = r.position();
        // Une copie du même fichier ailleurs (même nom, même taille, même passage) ne prend pas une place de plus.
        let nom = Path::new(&r.chemin).file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !copies.insert((nom, r.taille, position.clone())) {
            continue;
        }
        // Où ouvrir l'aperçu : la page, la ligne, le moment ; rien pour une photo ou un fichier entier.
        let repere = (!position.is_empty()).then_some(r.debut);
        let l = ligne(&r.chemin, position, r.extrait, false, repere);
        if deja.insert(l.cle.clone()) {
            lignes.push(l);
        }
        if lignes.len() >= CONTENU_MAX + NOMS_MAX + DOSSIERS_MAX {
            break;
        }
    }
    Ok(lignes)
}

/// Les dossiers qui contiennent plusieurs des fichiers trouvés (au plus `DOSSIERS_MAX`, les plus fournis).
fn dossiers_communs<'a>(chemins: impl Iterator<Item = &'a str>) -> Vec<(PathBuf, usize)> {
    let mut comptes: HashMap<&Path, usize> = HashMap::new();
    for c in chemins {
        if let Some(parent) = Path::new(c).parent() {
            *comptes.entry(parent).or_default() += 1;
        }
    }
    let mut dossiers: Vec<(PathBuf, usize)> =
        comptes.into_iter().filter(|(_, n)| *n >= DOSSIER_MIN).map(|(d, n)| (d.to_path_buf(), n)).collect();
    dossiers.sort_by(|a, b| b.1.cmp(&a.1));
    dossiers.truncate(DOSSIERS_MAX);
    dossiers
}

fn majuscule(prenom: &str) -> String {
    prenom
        .split(' ')
        .map(|m| {
            let mut c = m.chars();
            c.next().map(|p| p.to_uppercase().chain(c).collect::<String>()).unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Ouvre un fichier proposé, ou avec `dossier` le montre dans l'Explorateur ; la barre se ferme.
#[tauri::command]
pub fn ouvrir_fichier(app: AppHandle, cle: String, dossier: bool) -> Result<(), String> {
    let chemin = verrou(&etat().offerts).get(&cle).cloned().ok_or("fichier inconnu : cherchez à nouveau")?;
    historique::noter(&chemin);
    let ouvreur = app.opener();
    let resultat = if dossier {
        ouvreur.reveal_item_in_dir(&chemin)
    } else {
        ouvreur.open_path(chemin.to_string_lossy(), None::<&str>)
    };
    resultat.map_err(|e| e.to_string())?;
    raccourci::fermer(&app, false);
    Ok(())
}
