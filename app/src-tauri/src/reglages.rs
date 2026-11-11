//! Les réglages de Furet : les dossiers indexés et les options, dans `reglages.json`
//! (le dossier de configuration de l'appli). Portable : rien ici n'est propre à Windows.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;

/// Miroir de `Reglages` dans `src/ipc/reglages.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reglages {
    /// Les dossiers lus par leur contenu (et dont les noms sont cherchables).
    pub dossiers: Vec<PathBuf>,
    /// Lire le texte des images et des scans.
    pub ocr: bool,
    /// Transcrire la parole des audios et des vidéos.
    pub transcription: bool,
    /// Reconnaître les visages des photos (tout reste sur ce PC).
    pub visages: bool,
    /// Mettre l'indexation en pause quand le PC est sur batterie.
    pub pause_batterie: bool,
    /// … et pendant un jeu ou une appli en plein écran.
    pub pause_jeu: bool,
    /// Lire aussi le code des grandes bibliothèques (moteurs de jeu, SDK…), pas seulement leurs noms.
    pub bibliotheques: bool,
    /// Lancer Furet avec Windows.
    pub demarrage: bool,
    /// « auto » (la langue de Windows), « en » ou « fr ».
    pub langue: String,
    /// Le dossier des outils, modèles et données, s'il n'est pas celui par défaut (développement).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub racine: Option<PathBuf>,
}

impl Default for Reglages {
    fn default() -> Self {
        Reglages {
            dossiers: Vec::new(),
            ocr: true,
            transcription: true,
            visages: true,
            pause_batterie: true,
            pause_jeu: true,
            bibliotheques: false,
            demarrage: true,
            langue: "auto".into(),
            racine: None,
        }
    }
}

static REGLAGES: RwLock<Option<Reglages>> = RwLock::new(None);
static FICHIER: OnceLock<PathBuf> = OnceLock::new();
static PREMIER_LANCEMENT: AtomicBool = AtomicBool::new(false);
static APP: OnceLock<AppHandle> = OnceLock::new();

/// La langue des textes : celle du réglage, sinon celle du système (le français pour un Windows en français,
/// l'anglais pour tout le reste).
fn appliquer_langue(r: &Reglages) {
    use furet_moteur::langue::{Langue, definir};
    // Pour les essais : `FURET_LANGUE=en` ou `fr` passe avant le réglage.
    let forcee = std::env::var("FURET_LANGUE").ok();
    let francais = match forcee.as_deref().unwrap_or(r.langue.as_str()) {
        "fr" => true,
        "en" => false,
        _ => sys_locale::get_locale().is_some_and(|l| l.to_lowercase().starts_with("fr")),
    };
    definir(if francais { Langue::Francais } else { Langue::Anglais });
}

/// La langue des textes de l'interface : « fr » ou « en ».
#[tauri::command]
pub fn langue() -> &'static str {
    if furet_moteur::langue::francais() { "fr" } else { "en" }
}

/// Inscrit Furet au démarrage de Windows, ou l'en retire, selon le réglage (pas en mode de test).
pub fn appliquer_demarrage() {
    let Some(app) = APP.get() else { return };
    if std::env::var_os("FURET_TEST").is_some() {
        return;
    }
    let lancement = app.autolaunch();
    let voulu = lire().demarrage;
    if lancement.is_enabled().unwrap_or(!voulu) != voulu {
        let fait = if voulu { lancement.enable() } else { lancement.disable() };
        if let Err(e) = fait {
            log::warn!("démarrage avec Windows non changé : {e}");
        }
    }
}

/// Aucun réglage n'existait au lancement : c'est la première fois que Furet tourne sur ce PC.
pub fn premier_lancement() -> bool {
    PREMIER_LANCEMENT.load(Ordering::Relaxed)
}

/// Lit les réglages au lancement. La première fois : les dossiers habituels de l'utilisateur.
pub fn charger(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let fichier = app.path().app_config_dir().map(|d| d.join("reglages.json")).unwrap_or_else(|_| "reglages.json".into());
    let lus = std::fs::read_to_string(&fichier).ok().and_then(|t| serde_json::from_str::<Reglages>(&t).ok());
    let premiere_fois = lus.is_none();
    let reglages = lus.unwrap_or_else(|| Reglages { dossiers: dossiers_usuels(app), ..Default::default() });
    let _ = FICHIER.set(fichier);
    appliquer_langue(&reglages);
    *REGLAGES.write().unwrap_or_else(|e| e.into_inner()) = Some(reglages.clone());
    if premiere_fois {
        PREMIER_LANCEMENT.store(true, Ordering::Relaxed);
        enregistrer(&reglages);
    }
}

/// Documents, Images, Vidéos, Musique, Bureau et Téléchargements (ceux qui existent).
fn dossiers_usuels(app: &AppHandle) -> Vec<PathBuf> {
    let p = app.path();
    [p.document_dir(), p.picture_dir(), p.video_dir(), p.audio_dir(), p.desktop_dir(), p.download_dir()]
        .into_iter()
        .filter_map(Result::ok)
        .filter(|d| d.is_dir())
        .fold(Vec::new(), |mut tous, d| {
            ajouter_sans_doublon(&mut tous, d);
            tous
        })
}

pub fn lire() -> Reglages {
    REGLAGES.read().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

fn enregistrer(r: &Reglages) {
    let Some(fichier) = FICHIER.get() else { return };
    if let Some(dossier) = fichier.parent() {
        let _ = std::fs::create_dir_all(dossier);
    }
    match serde_json::to_string_pretty(r) {
        Ok(texte) => {
            if let Err(e) = std::fs::write(fichier, texte) {
                log::warn!("réglages non enregistrés : {e}");
            }
        }
        Err(e) => log::warn!("réglages non enregistrés : {e}"),
    }
}

/// Remplace les réglages, les enregistre, et prévient l'indexation.
fn changer(r: Reglages) -> Reglages {
    *REGLAGES.write().unwrap_or_else(|e| e.into_inner()) = Some(r.clone());
    enregistrer(&r);
    appliquer_langue(&r);
    appliquer_demarrage();
    crate::indexeur::reglages_changes();
    r
}

/// Ajoute `nouveau` sauf s'il est déjà dans un dossier de la liste ; retire ceux qu'il contient.
fn ajouter_sans_doublon(dossiers: &mut Vec<PathBuf>, nouveau: PathBuf) -> bool {
    if dossiers.iter().any(|d| nouveau.starts_with(d)) {
        return false;
    }
    dossiers.retain(|d| !d.starts_with(&nouveau));
    dossiers.push(nouveau);
    true
}

#[tauri::command]
pub fn reglages_lire() -> Reglages {
    lire()
}

/// Les options (les dossiers se changent avec `ajouter_dossier` et `retirer_dossier`).
#[tauri::command]
pub fn reglages_ecrire(reglages: Reglages) -> Reglages {
    let actuels = lire();
    changer(Reglages { dossiers: actuels.dossiers, racine: actuels.racine, ..reglages })
}

/// Choisit un dossier à indexer avec le sélecteur du système.
#[tauri::command]
pub async fn ajouter_dossier(app: AppHandle) -> Result<Reglages, String> {
    let titre = furet_moteur::langue::t("Dossier à indexer", "Folder to index");
    let Some(choisi) = app.dialog().file().set_title(titre).blocking_pick_folder() else {
        return Ok(lire());
    };
    let chemin = choisi.into_path().map_err(|e| e.to_string())?;
    let mut r = lire();
    if !ajouter_sans_doublon(&mut r.dossiers, chemin) {
        return Err(furet_moteur::langue::t(
            "Ce dossier est déjà indexé (il est dans un dossier de la liste).",
            "This folder is already indexed (it is inside a folder of the list).",
        )
        .into());
    }
    Ok(changer(r))
}

/// Retire un dossier : ses fichiers sortent de l'index.
#[tauri::command]
pub fn retirer_dossier(chemin: String) -> Reglages {
    let mut r = lire();
    r.dossiers.retain(|d| d != Path::new(&chemin));
    let r = changer(r);
    crate::indexeur::oublier(PathBuf::from(chemin));
    r
}
