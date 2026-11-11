//! Le premier lancement : où Furet range ses données, et le téléchargement de ses outils et modèles (voir
//! `furet_moteur::installation`). Tant que l'essentiel manque, l'île ouvre l'accueil au lieu de la barre.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::{Duration, Instant};

use furet_moteur::installation::{self, Groupe, Paquet};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::reglages;

pub const EVT_INSTALLATION: &str = "furet://installation";
/// L'interface reçoit l'avancement au plus 5 fois par seconde.
const PUBLICATION_MIN: Duration = Duration::from_millis(200);
/// Après une coupure, on réessaie seul à ce rythme (le téléchargement reprend où il en était).
const REESSAI: Duration = Duration::from_secs(20);
const ESSAIS_MAX: u32 = 5;

static RACINE: OnceLock<PathBuf> = OnceLock::new();
static APP: OnceLock<AppHandle> = OnceLock::new();
static EN_COURS: AtomicBool = AtomicBool::new(false);
static ETAT: LazyLock<Mutex<(Etat, Option<Instant>)>> = LazyLock::new(|| Mutex::new((Etat::Pret, None)));

/// Miroir de `EtatInstallation` dans `src/ipc/installation.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Etat {
    /// Tout ce qu'il faut est là.
    Pret,
    /// Il manque des choses : octets à télécharger pour l'essentiel, la transcription, les visages.
    AInstaller { essentiel: u64, transcription: u64, visages: u64 },
    Telechargement { fait: u64, total: u64, nom: String },
    Erreur { message: String },
}

/// Le dossier de Furet : `FURET_RACINE`, sinon celui des réglages (développement), sinon les données locales
/// de l'utilisateur (`%LOCALAPPDATA%\app.furet`). Les données de l'index sont dans `donnees/`.
pub fn choisir_racine(app: &AppHandle) -> PathBuf {
    let racine = std::env::var_os("FURET_RACINE")
        .map(PathBuf::from)
        .or_else(|| reglages::lire().racine)
        .or_else(|| app.path().app_local_data_dir().ok())
        .unwrap_or_else(|| PathBuf::from("furet"));
    let _ = APP.set(app.clone());
    let _ = RACINE.set(racine.clone());
    racine
}

fn racine() -> &'static Path {
    RACINE.get().map_or(Path::new("furet"), PathBuf::as_path)
}

fn octets(groupe: Groupe) -> u64 {
    installation::manquants(racine(), &[groupe]).iter().map(|p| p.taille).sum()
}

/// L'essentiel est là : la recherche et l'indexation peuvent tourner.
pub fn essentiel_pret() -> bool {
    installation::manquants(racine(), &[Groupe::Essentiel]).is_empty()
}

pub fn groupe_pret(groupe: Groupe) -> bool {
    installation::manquants(racine(), &[groupe]).is_empty()
}

/// Les groupes voulus par les réglages.
fn voulus() -> Vec<Groupe> {
    let r = reglages::lire();
    let mut g = vec![Groupe::Essentiel];
    if r.transcription {
        g.push(Groupe::Transcription);
    }
    if r.visages {
        g.push(Groupe::Visages);
    }
    g
}

fn etat_calcule() -> Etat {
    if voulus().iter().all(|g| groupe_pret(*g)) {
        Etat::Pret
    } else {
        Etat::AInstaller {
            essentiel: octets(Groupe::Essentiel),
            transcription: octets(Groupe::Transcription),
            visages: octets(Groupe::Visages),
        }
    }
}

fn publier(etat: Etat) {
    let mut garde = ETAT.lock().unwrap_or_else(|e| e.into_inner());
    let meme = std::mem::discriminant(&garde.0) == std::mem::discriminant(&etat);
    let trop_tot = garde.1.is_some_and(|t| t.elapsed() < PUBLICATION_MIN);
    garde.0 = etat.clone();
    if meme && trop_tot {
        return;
    }
    garde.1 = Some(Instant::now());
    drop(garde);
    if let Some(app) = APP.get() {
        let _ = app.emit(EVT_INSTALLATION, etat);
    }
}

#[tauri::command]
pub fn etat_installation() -> Etat {
    if EN_COURS.load(Ordering::Relaxed) {
        return ETAT.lock().unwrap_or_else(|e| e.into_inner()).0.clone();
    }
    etat_calcule()
}

/// Télécharge ce qui manque (selon les réglages), sur un fil à part. Sans effet si c'est déjà en cours.
#[tauri::command]
pub fn installer() {
    if EN_COURS.swap(true, Ordering::Relaxed) {
        return;
    }
    let lance = std::thread::Builder::new().name("furet-installation".into()).spawn(|| {
        let arret = Arc::new(AtomicBool::new(false));
        let mut essais = 0;
        let resultat = loop {
            let paquets: Vec<Paquet> = installation::manquants(racine(), &voulus());
            if paquets.is_empty() {
                break Ok(());
            }
            match installation::installer(racine(), &paquets, &arret, &mut |a| {
                publier(Etat::Telechargement { fait: a.fait, total: a.total, nom: a.nom.into() })
            }) {
                Ok(()) => break Ok(()),
                // Connexion coupée, fichier abîmé : on réessaie un peu plus tard, la suite reprend.
                Err(e) if essais + 1 < ESSAIS_MAX => {
                    essais += 1;
                    log::warn!("installation interrompue ({essais}/{ESSAIS_MAX}) : {e:#}");
                    let suite = furet_moteur::langue::t("nouvel essai dans quelques secondes", "trying again in a few seconds");
                    publier(Etat::Erreur { message: format!("{e:#} — {suite}") });
                    std::thread::sleep(REESSAI);
                }
                Err(e) => break Err(e),
            }
        };
        EN_COURS.store(false, Ordering::Relaxed);
        match resultat {
            Ok(()) => {
                log::info!("installation terminée");
                publier(etat_calcule());
                // La recherche attend l'essentiel ; l'indexation repart avec ce qui vient d'arriver.
                crate::indexeur::reglages_changes();
            }
            Err(e) => {
                log::error!("installation : {e:#}");
                publier(Etat::Erreur { message: format!("{e:#}") });
            }
        }
    });
    if lance.is_err() {
        EN_COURS.store(false, Ordering::Relaxed);
    }
}

/// Attend que l'essentiel soit là (vérifié toutes les 2 secondes).
pub fn attendre_essentiel() {
    while !essentiel_pret() {
        std::thread::sleep(Duration::from_secs(2));
    }
}
