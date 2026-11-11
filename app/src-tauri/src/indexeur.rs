//! L'indexation en arrière-plan. Un gardien la lance (au démarrage, quand les réglages changent, puis
//! toutes les 30 minutes pour attraper les nouveaux fichiers) et l'arrête proprement dès qu'il faut :
//! batterie, jeu ou plein écran, pause demandée. L'arrêter ferme aussi son llama-server : la carte
//! graphique est libérée pendant un jeu. Rien n'est perdu : la fois suivante, elle reprend où elle en était.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

use furet_moteur::indexation::{self, Progres};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::{installation, plateforme, recherche, reglages};
use furet_moteur::installation::Groupe;

pub const EVT_ETAT: &str = "furet://indexation";
/// Le lancement de l'appli reste léger : l'indexation attend un peu.
const ATTENTE_AU_DEMARRAGE: Duration = Duration::from_secs(8);
/// Pour attraper les fichiers nouveaux ou modifiés (en attendant de suivre le disque en direct, étape 5).
const INTERVALLE: Duration = Duration::from_secs(30 * 60);
/// Après une erreur, on réessaie plus tard.
const APRES_ERREUR: Duration = Duration::from_secs(5 * 60);
/// Batterie, jeu, pause : vérifiés à ce rythme pendant l'indexation.
const VERIFICATION: Duration = Duration::from_secs(3);
/// L'interface reçoit l'avancement au plus 4 fois par seconde.
const PUBLICATION_MIN: Duration = Duration::from_millis(250);

/// Miroir de `EtatIndexation` dans `src/ipc/indexation.ts`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum Etat {
    /// Pas encore commencé, ou aucun dossier.
    Attente,
    Parcours,
    Lecture { faits: usize, total: usize },
    Visages { faits: usize, total: usize },
    Transcription { faits: usize, total: usize },
    Pause { raison: String },
    AJour,
    Erreur { message: String },
}

struct Partage {
    etat: Etat,
    publie: Option<Instant>,
    /// Quelque chose a changé (réglages, « indexer maintenant ») : relancer sans attendre.
    demande: bool,
    /// L'indexation en cours : la mettre à vrai l'arrête.
    arret: Option<Arc<AtomicBool>>,
}

static PARTAGE: LazyLock<(Mutex<Partage>, Condvar)> =
    LazyLock::new(|| (Mutex::new(Partage { etat: Etat::Attente, publie: None, demande: false, arret: None }), Condvar::new()));
static PAUSE_MANUELLE: AtomicBool = AtomicBool::new(false);

fn partage() -> std::sync::MutexGuard<'static, Partage> {
    PARTAGE.0.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn demarrer(app: &AppHandle) {
    // Mode de test (index d'essai) : rien n'est indexé, et la barre n'affiche aucun état d'indexation.
    if std::env::var_os("FURET_TEST").is_some() {
        return;
    }
    let app = app.clone();
    let lance = std::thread::Builder::new().name("furet-indexation".into()).spawn(move || gardien(&app));
    if let Err(e) = lance {
        log::error!("l'indexation n'a pas pu démarrer : {e}");
    }
}

/// Publie l'état à l'interface (les étapes d'avancement au plus 4 fois par seconde).
fn publier(app: &AppHandle, etat: Etat) {
    let mut p = partage();
    let meme_phase = std::mem::discriminant(&p.etat) == std::mem::discriminant(&etat);
    let trop_tot = p.publie.is_some_and(|t| t.elapsed() < PUBLICATION_MIN);
    let fin = matches!(etat, Etat::Lecture { faits, total } | Etat::Visages { faits, total }
        | Etat::Transcription { faits, total } if faits == total);
    p.etat = etat.clone();
    if meme_phase && trop_tot && !fin {
        return;
    }
    p.publie = Some(Instant::now());
    drop(p);
    let _ = app.emit(EVT_ETAT, etat);
}

/// Pourquoi il faut s'arrêter maintenant, s'il le faut.
fn raison_de_pause(r: &reglages::Reglages) -> Option<&'static str> {
    if PAUSE_MANUELLE.load(Ordering::Relaxed) {
        Some("en pause")
    } else if r.pause_batterie && plateforme::sur_batterie() {
        Some("sur batterie")
    } else if r.pause_jeu && plateforme::jeu_en_cours() {
        Some("jeu ou plein écran")
    } else {
        None
    }
}

/// Attend au plus `duree`, ou qu'on demande une indexation. Vrai si elle a été demandée.
fn attendre(duree: Duration) -> bool {
    let (verrou, cond) = &*PARTAGE;
    let p = verrou.lock().unwrap_or_else(|e| e.into_inner());
    let (mut p, _) = cond.wait_timeout_while(p, duree, |p| !p.demande).unwrap_or_else(|e| e.into_inner());
    std::mem::take(&mut p.demande)
}

fn gardien(app: &AppHandle) {
    let mut demande = attendre(ATTENTE_AU_DEMARRAGE);
    let mut prochaine = Instant::now();
    loop {
        let r = reglages::lire();
        // Premier lancement : rien à faire tant que les outils et le modèle ne sont pas téléchargés.
        if !installation::essentiel_pret() {
            publier(app, Etat::Attente);
            demande |= attendre(Duration::from_secs(5));
            continue;
        }
        // Un réglage vient d'activer la transcription ou les visages : leurs modèles arrivent en arrière-plan.
        if matches!(installation::etat_installation(), installation::Etat::AInstaller { .. }) {
            installation::installer();
        }
        if let Some(raison) = raison_de_pause(&r) {
            publier(app, Etat::Pause { raison: raison.into() });
            demande |= attendre(VERIFICATION);
            continue;
        }
        if r.dossiers.is_empty() {
            publier(app, Etat::Attente);
            demande = attendre(Duration::from_secs(3600));
            continue;
        }
        if !demande && Instant::now() < prochaine {
            demande = attendre(prochaine - Instant::now());
            continue;
        }
        demande = false;

        let arret = Arc::new(AtomicBool::new(false));
        partage().arret = Some(arret.clone());
        // Le guetteur arrête l'indexation dès qu'une raison de pause apparaît.
        let fini = Arc::new(AtomicBool::new(false));
        {
            let (arret, fini) = (arret.clone(), fini.clone());
            std::thread::spawn(move || {
                while !fini.load(Ordering::Relaxed) {
                    std::thread::sleep(VERIFICATION);
                    if raison_de_pause(&reglages::lire()).is_some() {
                        arret.store(true, Ordering::Relaxed);
                    }
                }
            });
        }
        let suivi_app = app.clone();
        let suivi: indexation::Suivi = Arc::new(move |p: Progres| {
            publier(
                &suivi_app,
                match p {
                    Progres::Parcours => Etat::Parcours,
                    Progres::Lecture { faits, total } => Etat::Lecture { faits, total },
                    Progres::Visages { faits, total } => Etat::Visages { faits, total },
                    Progres::Transcription { faits, total } => Etat::Transcription { faits, total },
                },
            )
        });
        let (racine, base) = recherche::racine_et_base();
        let debut = Instant::now();
        let resultat = indexation::indexer(
            &racine,
            &base,
            &r.dossiers,
            indexation::Options {
                cpu: false,
                tout: false,
                avec_ocr: r.ocr,
                avec_transcription: r.transcription && installation::groupe_pret(Groupe::Transcription),
                avec_visages: r.visages && installation::groupe_pret(Groupe::Visages),
                fils: 0,
                jetons_image: None,
                arret: arret.clone(),
                suivi: Some(suivi),
                bibliotheques: r.bibliotheques,
                noms: false,
            },
        );
        fini.store(true, Ordering::Relaxed);
        partage().arret = None;
        recherche::rafraichir_si_change();
        match resultat {
            // Arrêtée (pause, réglages changés) : elle reprendra dès que possible, là où elle en était.
            Ok(()) if arret.load(Ordering::Relaxed) => {
                log::info!("indexation interrompue après {} s", debut.elapsed().as_secs());
                demande = true;
            }
            Ok(()) => {
                log::info!("indexation à jour en {} s", debut.elapsed().as_secs());
                publier(app, Etat::AJour);
                prochaine = Instant::now() + INTERVALLE;
            }
            Err(e) => {
                log::error!("indexation : {e:#}");
                publier(app, Etat::Erreur { message: format!("{e:#}") });
                prochaine = Instant::now() + APRES_ERREUR;
            }
        }
    }
}

/// Les réglages ont changé : l'indexation en cours s'arrête et repart avec les nouveaux.
pub fn reglages_changes() {
    let (_, cond) = &*PARTAGE;
    let mut p = partage();
    if let Some(a) = &p.arret {
        a.store(true, Ordering::Relaxed);
    }
    p.demande = true;
    cond.notify_all();
}

/// Un dossier retiré des réglages : ses fichiers sortent de l'index (sur un fil à part).
pub fn oublier(dossier: PathBuf) {
    std::thread::spawn(move || {
        let (_, base) = recherche::racine_et_base();
        let r = furet_moteur::base::ouvrir(&base)
            .and_then(|mut b| furet_moteur::base::oublier_dossier(&mut b, &dossier.to_string_lossy()));
        if let Err(e) = r {
            log::warn!("dossier retiré mais pas oublié de l'index : {e:#}");
        }
        recherche::rafraichir_si_change();
    });
}

#[tauri::command]
pub fn etat_indexation() -> Etat {
    partage().etat.clone()
}

/// Relance l'indexation tout de suite (nouveaux fichiers).
#[tauri::command]
pub fn indexer_maintenant() {
    PAUSE_MANUELLE.store(false, Ordering::Relaxed);
    reglages_changes();
}

/// Met l'indexation en pause, ou la reprend.
#[tauri::command]
pub fn basculer_pause(app: AppHandle) -> Etat {
    let pause = !PAUSE_MANUELLE.fetch_xor(true, Ordering::Relaxed);
    if pause {
        if let Some(a) = &partage().arret {
            a.store(true, Ordering::Relaxed);
        }
        publier(&app, Etat::Pause { raison: "en pause".into() });
    } else {
        reglages_changes();
    }
    etat_indexation()
}
