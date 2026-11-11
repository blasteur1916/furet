//! Les mises à jour : vérifiées une minute après le lancement puis toutes les 6 heures sur la page des
//! versions de Furet. Une nouvelle version est téléchargée en arrière-plan et vérifiée (signature), puis
//! installée quand l'utilisateur le demande (« Redémarrer pour mettre à jour », dans les réglages).

use std::sync::Mutex;
use std::time::Duration;

use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::{Update, UpdaterExt};

pub const EVT_MISE_A_JOUR: &str = "furet://mise-a-jour";
const PREMIERE_VERIFICATION: Duration = Duration::from_secs(60);
const INTERVALLE: Duration = Duration::from_secs(6 * 3600);

/// La version téléchargée, prête à être installée.
static PRETE: Mutex<Option<(Update, Vec<u8>)>> = Mutex::new(None);

pub fn demarrer(app: &AppHandle) {
    // Mode de test : pas de mise à jour.
    if std::env::var_os("FURET_TEST").is_some() {
        return;
    }
    let app = app.clone();
    let lance = std::thread::Builder::new().name("furet-mises-a-jour".into()).spawn(move || {
        std::thread::sleep(PREMIERE_VERIFICATION);
        loop {
            if PRETE.lock().unwrap_or_else(|e| e.into_inner()).is_none()
                && let Err(e) = tauri::async_runtime::block_on(verifier(&app))
            {
                // Hors ligne, page des versions pas encore publiée… on réessaiera.
                log::info!("mises à jour : {e}");
            }
            std::thread::sleep(INTERVALLE);
        }
    });
    if let Err(e) = lance {
        log::warn!("les mises à jour ne seront pas vérifiées : {e}");
    }
}

async fn verifier(app: &AppHandle) -> tauri_plugin_updater::Result<()> {
    let Some(update) = app.updater()?.check().await? else { return Ok(()) };
    log::info!("mise à jour {} disponible : téléchargement", update.version);
    let octets = update.download(|_, _| {}, || {}).await?;
    let version = update.version.clone();
    *PRETE.lock().unwrap_or_else(|e| e.into_inner()) = Some((update, octets));
    let _ = app.emit(EVT_MISE_A_JOUR, version);
    Ok(())
}

/// La version prête à être installée, s'il y en a une.
#[tauri::command]
pub fn mise_a_jour_prete() -> Option<String> {
    PRETE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(u, _)| u.version.clone())
}

/// Installe la version téléchargée : l'index est sauvegardé, l'installateur remplace Furet et le relance.
#[tauri::command]
pub fn installer_mise_a_jour(app: AppHandle) -> Result<(), String> {
    let Some((update, octets)) = PRETE.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return Err(furet_moteur::langue::t("aucune mise à jour prête", "no update ready").into());
    };
    crate::recherche::arreter();
    update.install(octets).map_err(|e| e.to_string())?;
    app.restart();
}
