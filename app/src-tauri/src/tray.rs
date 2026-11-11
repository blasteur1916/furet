//! L'icône de la zone de notification : la seule façon de quitter, l'île n'ayant pas de cadre.

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;

const QUITTER: &str = "quitter";

pub fn creer(app: &tauri::App) -> tauri::Result<()> {
    let quitter = MenuItem::with_id(app, QUITTER, furet_moteur::langue::t("Quitter Furet", "Quit Furet"), true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&quitter])?;
    let mut icone = TrayIconBuilder::with_id("principale")
        .tooltip(furet_moteur::langue::t("Furet — Alt+Espace pour chercher", "Furet — Alt+Space to search"))
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            if event.id() == QUITTER {
                log::info!("arrêt demandé depuis l'icône");
                app.exit(0);
            }
        });
    if let Some(image) = app.default_window_icon() {
        icone = icone.icon(image.clone());
    }
    icone.build(app)?;
    Ok(())
}
