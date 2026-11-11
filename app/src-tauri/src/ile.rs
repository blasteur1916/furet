//! La fenêtre de l'île (reprise de Quiera) : transparente, toujours au-dessus, en haut de l'écran
//! principal. Elle ne prend jamais le clavier d'elle-même. Windows limite son dessin et ses clics à la
//! région que l'interface envoie (la forme de l'île) : ailleurs, les clics vont aux fenêtres du dessous.
//! Fermée, la région est vide : l'île est invisible et entièrement traversée par les clics.

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use serde::Deserialize;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewWindow};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::{CombineRgn, CreateRectRgn, DeleteObject, RGN_OR, SetWindowRgn};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, HWND_TOPMOST, SET_WINDOW_POS_FLAGS, SW_SHOWNOACTIVATE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_EX_APPWINDOW,
    WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

pub const LABEL: &str = "ile";
/// Assez grand pour la barre, ses résultats et (plus tard) le panneau d'aperçu, ombre et « flares » compris.
const TAILLE_LOGIQUE: (f64, f64) = (900.0, 640.0);
/// L'interface envoie quelques rectangles ; au-delà, c'est une erreur.
const RECTANGLES_MAX: usize = 16;

/// Un rectangle en pixels CSS de la fenêtre, tel que l'interface l'envoie.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

static HWND_ILE: AtomicIsize = AtomicIsize::new(0);

pub fn hwnd() -> Option<HWND> {
    match HWND_ILE.load(Ordering::Relaxed) {
        0 => None,
        brut => Some(HWND(brut as *mut c_void)),
    }
}

/// Place la fenêtre, lui donne ses styles natifs, la rend invisible (région vide) et l'affiche.
pub fn installer(app: &AppHandle) -> tauri::Result<()> {
    let fenetre = app.get_webview_window(LABEL).expect("la fenêtre de l'île est déclarée dans tauri.conf.json");
    placer(&fenetre)?;
    let h = fenetre.hwnd()?;
    HWND_ILE.store(h.0 as isize, Ordering::Relaxed);
    appliquer_region(h, &[]);
    // SAFETY : `h` est une fenêtre vivante de ce programme.
    unsafe {
        poser_styles(h, SWP_FRAMECHANGED);
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
    }
    Ok(())
}

/// En haut de l'écran principal, centrée, à l'échelle de l'écran.
pub fn placer(fenetre: &WebviewWindow) -> tauri::Result<()> {
    let Some(ecran) = fenetre.primary_monitor()? else {
        log::warn!("aucun écran : l'île reste où elle est");
        return Ok(());
    };
    let echelle = ecran.scale_factor();
    let largeur = (TAILLE_LOGIQUE.0 * echelle).round() as i32;
    let hauteur = (TAILLE_LOGIQUE.1 * echelle).round() as i32;
    let (pos, taille) = (ecran.position(), ecran.size());
    fenetre.set_size(PhysicalSize::new(largeur as u32, hauteur as u32))?;
    fenetre.set_position(PhysicalPosition::new(pos.x + (taille.width as i32 - largeur) / 2, pos.y))?;
    Ok(())
}

/// Les styles de l'île, en gardant les autres : jamais active d'elle-même (NOACTIVATE), pas de bouton
/// dans la barre des tâches ni dans Alt+Tab (TOOLWINDOW), et LAYERED, sans quoi la région rendrait
/// opaques les pixels transparents. Puis tout en haut de la pile des fenêtres.
///
/// SAFETY : `h` doit être une fenêtre de ce programme (une poignée périmée fait seulement échouer les appels).
unsafe fn poser_styles(h: HWND, en_plus: SET_WINDOW_POS_FLAGS) {
    unsafe {
        let actuel = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        let voulu = (actuel | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_LAYERED.0) & !WS_EX_APPWINDOW.0;
        if voulu != actuel {
            SetWindowLongPtrW(h, GWL_EXSTYLE, voulu as isize);
        }
        if let Err(e) = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | en_plus) {
            log::warn!("l'île n'a pas pu passer au-dessus : {e}");
        }
    }
}

/// Repasse au-dessus des autres fenêtres « toujours au-dessus » (Gestionnaire des tâches, lecteurs…),
/// et repose les styles, que l'interpréteur de commandes enlève parfois.
pub fn remettre_au_dessus() {
    if let Some(h) = hwnd() {
        // SAFETY : voir `poser_styles`.
        unsafe { poser_styles(h, SET_WINDOW_POS_FLAGS(0)) };
    }
}

/// `rects` en pixels physiques de la fenêtre, arrondis vers l'extérieur pour ne rien couper.
fn appliquer_region(h: HWND, rects: &[Rect]) {
    // SAFETY : les régions sont créées et libérées ici ; après un SetWindowRgn réussi, Windows
    // possède `region`, qui n'est donc libérée qu'en cas d'échec.
    unsafe {
        let region = CreateRectRgn(0, 0, 0, 0);
        for r in rects {
            let morceau = CreateRectRgn(
                r.x.floor() as i32,
                r.y.floor() as i32,
                (r.x + r.width).ceil() as i32,
                (r.y + r.height).ceil() as i32,
            );
            CombineRgn(Some(region), Some(region), Some(morceau), RGN_OR);
            let _ = DeleteObject(morceau.into());
        }
        if SetWindowRgn(h, Some(region), true) == 0 {
            log::warn!("SetWindowRgn a échoué");
            let _ = DeleteObject(region.into());
        }
    }
}

/// Limite le dessin et les clics de l'île à `rects` (pixels CSS). Une liste vide la rend invisible.
#[tauri::command]
pub fn ile_region(fenetre: WebviewWindow, rects: Vec<Rect>) -> Result<(), String> {
    if rects.len() > RECTANGLES_MAX {
        return Err(format!("trop de rectangles : {}", rects.len()));
    }
    let echelle = fenetre.scale_factor().map_err(|e| e.to_string())?;
    let physiques: Vec<Rect> = rects
        .iter()
        .filter(|r| [r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite()) && r.width > 0.0 && r.height > 0.0)
        .map(|r| Rect { x: r.x * echelle, y: r.y * echelle, width: r.width * echelle, height: r.height * echelle })
        .collect();
    let h = fenetre.hwnd().map_err(|e| e.to_string())?;
    appliquer_region(h, &physiques);
    Ok(())
}

/// Île fermée : WebView2 libère ce qu'il peut (caches, mémoire graphique) ; normale dès qu'elle s'ouvre.
#[tauri::command]
pub fn ile_memoire(fenetre: WebviewWindow, basse: bool) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
    };
    use windows::core::Interface;
    let niveau = if basse { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
    let _ = fenetre.with_webview(move |webview| {
        // SAFETY : appels COM sur le fil de la WebView, dans with_webview.
        unsafe {
            if let Ok(coeur) = webview.controller().CoreWebView2().and_then(|c| c.cast::<ICoreWebView2_19>()) {
                let _ = coeur.SetMemoryUsageTargetLevel(niveau);
            }
        }
    });
}
