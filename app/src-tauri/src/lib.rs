//! Furet : une barre de recherche qui descend du haut de l'écran (Alt+Espace) et retrouve un fichier par
//! son nom ou par ce qu'il contient, entièrement sur le PC.

mod actions;
mod apercus;
mod historique;
mod ile;
mod indexeur;
mod installation;
mod miniatures;
mod mises_a_jour;
mod plateforme;
mod raccourci;
mod recherche;
mod reglages;
mod tray;

use tauri::{Manager, RunEvent, WindowEvent};

pub fn run() {
    // Si l'appli s'arrête, même brutalement, Windows arrête aussi ses programmes enfants (llama-server…).
    furet_moteur::processus::lier_au_programme();
    let app = tauri::Builder::default()
        // En premier : un deuxième lancement s'arrête avant de faire quoi que ce soit.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            raccourci::ligne_de_commande(app, &args);
        }))
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .timezone_strategy(tauri_plugin_log::TimezoneStrategy::UseLocal)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // Lancé avec Windows : « --demarrage » le dit (rien ne s'ouvre, Furet attend le raccourci).
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--demarrage"])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .register_asynchronous_uri_scheme_protocol(miniatures::SCHEMA, |_ctx, demande, repondeur| {
            let cle = demande.uri().path().trim_start_matches('/').to_owned();
            let grand = demande.uri().query() == Some("grand");
            miniatures::servir(recherche::chemin_offert(&cle), cle, grand, repondeur);
        })
        .register_asynchronous_uri_scheme_protocol(apercus::SCHEMA, |_ctx, demande, repondeur| {
            apercus::servir(demande, repondeur);
        })
        .setup(|app| {
            reglages::charger(app.handle());
            historique::charger(app.handle());
            tray::creer(app)?;
            ile::installer(app.handle())?;
            let racine = installation::choisir_racine(app.handle());
            recherche::demarrer(racine);
            raccourci::demarrer(app.handle());
            indexeur::demarrer(app.handle());
            reglages::appliquer_demarrage();
            mises_a_jour::demarrer(app.handle());
            // Premier lancement, ou outils pas encore téléchargés : l'accueil s'ouvre de lui-même.
            if reglages::premier_lancement() || !installation::essentiel_pret() {
                let app = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(1200));
                    raccourci::ouvrir_au_lancement(&app);
                });
            }
            Ok(())
        })
        .on_window_event(|fenetre, evenement| {
            if let WindowEvent::ScaleFactorChanged { .. } = evenement
                && let Some(ile) = fenetre.app_handle().get_webview_window(ile::LABEL)
                && let Err(e) = ile::placer(&ile)
            {
                log::warn!("l'île n'a pas pu être replacée après un changement d'échelle : {e}");
            }
        })
        .invoke_handler(tauri::generate_handler![
            ile::ile_region,
            ile::ile_memoire,
            raccourci::fermer_barre,
            recherche::chercher_noms,
            recherche::chercher_contenu,
            recherche::ouvrir_fichier,
            recherche::etat_index,
            recherche::filtres,
            recherche::recents,
            installation::etat_installation,
            installation::installer,
            raccourci::raccourci_actuel,
            mises_a_jour::mise_a_jour_prete,
            mises_a_jour::installer_mise_a_jour,
            apercus::apercu,
            actions::copier_chemin,
            actions::ouvrir_avec,
            actions::glisser,
            reglages::reglages_lire,
            reglages::reglages_ecrire,
            reglages::ajouter_dossier,
            reglages::retirer_dossier,
            indexeur::etat_indexation,
            indexeur::indexer_maintenant,
            indexeur::basculer_pause,
        ])
        .build(tauri::generate_context!())
        .expect("Furet n'a pas pu démarrer");
    app.run(|_app, evenement| {
        if let RunEvent::Exit = evenement {
            recherche::arreter();
        }
    });
}
